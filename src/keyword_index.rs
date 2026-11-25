//! A rebuildable, persistent cache. Original image metadata remains authoritative.
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::Path,
    time::{Duration, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileStamp {
    len: u64,
    modified: Option<String>,
}
impl FileStamp {
    fn new(len: u64, modified: Option<std::time::SystemTime>) -> Self {
        let modified = modified.map(|time| match time.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_nanos().to_string(),
            Err(e) => format!("-{}", e.duration().as_nanos()),
        });
        Self { len, modified }
    }
    fn from_metadata(metadata: fs::Metadata) -> Self {
        Self::new(metadata.len(), metadata.modified().ok())
    }
}
#[cfg(test)]
thread_local! {
    /// `Fingerprint::capture` calls on this thread (tests).
    pub static CAPTURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    image: FileStamp,
    sidecar: Option<FileStamp>,
}
impl Fingerprint {
    pub fn capture(path: &Path) -> Result<Self> {
        #[cfg(test)]
        CAPTURES.with(|c| c.set(c.get() + 1));
        let image = FileStamp::from_metadata(fs::metadata(path)?);
        let sidecar = match fs::metadata(crate::metadata::sidecar(path)) {
            Ok(m) => Some(FileStamp::from_metadata(m)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Self { image, sidecar })
    }
    /// The fingerprint as the directory listing gave it (no system call of
    /// its own). Only compared with stored fingerprints, never stored: an
    /// entry may lag behind a file another program still has open.
    pub fn listed(image: crate::model::Listed, sidecar: Option<crate::model::Listed>) -> Self {
        Self {
            image: FileStamp::new(image.len, image.modified),
            sidecar: sidecar.map(|s| FileStamp::new(s.len, s.modified)),
        }
    }
    pub fn reusable(&self) -> bool {
        self.image.modified.is_some() && self.sidecar.as_ref().is_none_or(|s| s.modified.is_some())
    }
}
#[derive(Clone, Debug)]
pub struct CachedKeywords {
    pub stamp: Fingerprint,
    pub tags: BTreeSet<String>,
}
pub fn key(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}
pub struct KeywordIndex {
    connection: Connection,
}
impl KeywordIndex {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path).context("Cannot open keyword index")?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(
            version <= 1,
            "Keyword index belongs to a newer NoirView version"
        );
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
            CREATE TABLE IF NOT EXISTS keyword_index (
                folder BLOB NOT NULL, recursive INTEGER NOT NULL, path BLOB NOT NULL,
                stamp TEXT NOT NULL, tags TEXT NOT NULL,
                PRIMARY KEY(folder,recursive,path)
            ) WITHOUT ROWID;
            CREATE INDEX IF NOT EXISTS keyword_index_path ON keyword_index(path);
            PRAGMA user_version=1;",
        )?;
        // Metadata reader revision is independent of the durable user database.
        // Re-read unchanged JPEGs when support for another keyword field is added.
        let tx = connection.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS reader_revision (revision INTEGER NOT NULL);",
        )?;
        let revision: Option<i64> = tx
            .query_row("SELECT revision FROM reader_revision LIMIT 1", [], |r| {
                r.get(0)
            })
            .optional()?;
        if revision != Some(3) {
            tx.execute_batch("DELETE FROM keyword_index; DELETE FROM reader_revision; INSERT INTO reader_revision VALUES(3);")?;
        }
        tx.commit()?;
        Ok(Self { connection })
    }
    pub fn load(&self, folder: &Path, recursive: bool) -> Result<HashMap<Vec<u8>, CachedKeywords>> {
        let mut query = self.connection.prepare(
            "SELECT path,stamp,tags FROM keyword_index WHERE folder=?1 AND recursive=?2",
        )?;
        let rows = query.query_map(params![key(folder), recursive], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut result = HashMap::new();
        for row in rows {
            let (path, stamp, tags) = row?;
            result.insert(
                path,
                CachedKeywords {
                    stamp: serde_json::from_str(&stamp)?,
                    tags: serde_json::from_str(&tags)?,
                },
            );
        }
        Ok(result)
    }
    /// Rows for every file under `folder`, whichever scope stored them (the
    /// folder watcher stores under the watched folder and its subfolder
    /// setting, the viewer under the folder it opened). The rows of this
    /// scope come first; each row is still checked against the file.
    pub fn load_any(
        &self,
        folder: &Path,
        recursive: bool,
    ) -> Result<HashMap<Vec<u8>, CachedKeywords>> {
        let mut result = self.load(folder, recursive)?;
        let mut low = key(folder);
        if low.last() != Some(&(std::path::MAIN_SEPARATOR as u8)) {
            low.push(std::path::MAIN_SEPARATOR as u8);
        }
        let mut high = low.clone();
        *high.last_mut().unwrap() += 1;
        let mut query = self.connection.prepare_cached(
            "SELECT path,stamp,tags FROM keyword_index WHERE path >= ?1 AND path < ?2",
        )?;
        let rows = query.query_map(params![low, high], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (path, stamp, tags) = row?;
            if result.contains_key(&path) {
                continue;
            }
            result.insert(
                path,
                CachedKeywords {
                    stamp: serde_json::from_str(&stamp)?,
                    tags: serde_json::from_str(&tags)?,
                },
            );
        }
        Ok(result)
    }
    /// A current row for one file from any scope.
    pub fn get_any(&self, path: &Path, stamp: &Fingerprint) -> Result<Option<CachedKeywords>> {
        let mut query = self
            .connection
            .prepare_cached("SELECT stamp,tags FROM keyword_index WHERE path=?1")?;
        let rows = query.query_map(params![key(path)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (found, tags) = row?;
            let found: Fingerprint = serde_json::from_str(&found)?;
            if &found == stamp {
                return Ok(Some(CachedKeywords {
                    stamp: found,
                    tags: serde_json::from_str(&tags)?,
                }));
            }
        }
        Ok(None)
    }
    pub fn store(
        &mut self,
        folder: &Path,
        recursive: bool,
        rows: &[(Vec<u8>, CachedKeywords)],
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        {
            let mut query=tx.prepare_cached("INSERT INTO keyword_index(folder,recursive,path,stamp,tags) VALUES(?1,?2,?3,?4,?5)
                ON CONFLICT(folder,recursive,path) DO UPDATE SET stamp=excluded.stamp,tags=excluded.tags")?;
            for (path, row) in rows {
                query.execute(params![
                    key(folder),
                    recursive,
                    path,
                    serde_json::to_string(&row.stamp)?,
                    serde_json::to_string(&row.tags)?
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn prune(
        &mut self,
        folder: &Path,
        recursive: bool,
        valid: &HashSet<Vec<u8>>,
    ) -> Result<()> {
        let old = self.load(folder, recursive)?;
        let tx = self.connection.transaction()?;
        for path in old.keys().filter(|p| !valid.contains(*p)) {
            tx.execute(
                "DELETE FROM keyword_index WHERE folder=?1 AND recursive=?2 AND path=?3",
                params![key(folder), recursive, path],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn remove(&self, folder: &Path, recursive: bool, path: &Path) -> Result<()> {
        self.connection.execute(
            "DELETE FROM keyword_index WHERE folder=?1 AND recursive=?2 AND path=?3",
            params![key(folder), recursive, key(path)],
        )?;
        Ok(())
    }
    pub fn clear(&self, folder: &Path, recursive: bool) -> Result<()> {
        self.connection.execute(
            "DELETE FROM keyword_index WHERE folder=?1 AND recursive=?2",
            params![key(folder), recursive],
        )?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;
    #[test]
    fn reader_upgrade_discards_old_rows_once_without_touching_user_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.sqlite3");
        let mut index = KeywordIndex::open(&db).unwrap();
        let image = dir.path().join("old.jpg");
        fs::write(&image, b"image").unwrap();
        let rows = [(
            key(&image),
            CachedKeywords {
                stamp: Fingerprint::capture(&image).unwrap(),
                tags: BTreeSet::new(),
            },
        )];
        index.store(dir.path(), false, &rows).unwrap();
        index.connection.execute_batch("UPDATE reader_revision SET revision=2; CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES('keep');").unwrap();
        drop(index);
        let mut upgraded = KeywordIndex::open(&db).unwrap();
        assert!(upgraded.load(dir.path(), false).unwrap().is_empty());
        assert_eq!(
            upgraded
                .connection
                .query_row("SELECT value FROM unrelated", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "keep"
        );
        upgraded.store(dir.path(), false, &rows).unwrap();
        drop(upgraded);
        assert_eq!(
            KeywordIndex::open(&db)
                .unwrap()
                .load(dir.path(), false)
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn cache_survives_restart_and_scopes_folders() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("night_日本語.png");
        fs::write(&image, b"pixels").unwrap();
        let stamp = Fingerprint::capture(&image).unwrap();
        let tags = BTreeSet::from(["night & city".into(), "日本語".into()]);
        let db = dir.path().join("index.sqlite3");
        {
            let mut index = KeywordIndex::open(&db).unwrap();
            index
                .store(
                    dir.path(),
                    false,
                    &[(
                        key(&image),
                        CachedKeywords {
                            stamp: stamp.clone(),
                            tags: tags.clone(),
                        },
                    )],
                )
                .unwrap();
        }
        let index = KeywordIndex::open(&db).unwrap();
        let rows = index.load(dir.path(), false).unwrap();
        assert_eq!(rows[&key(&image)].tags, tags);
        assert_eq!(rows[&key(&image)].stamp, stamp);
        assert!(index.load(dir.path(), true).unwrap().is_empty());
    }
    #[test]
    fn rows_are_found_by_path_whichever_scope_stored_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let (top, deep) = (root.join("a.jpg"), root.join("sub/b.jpg"));
        std::fs::write(&top, b"x").unwrap();
        std::fs::write(&deep, b"y").unwrap();
        let mut index = KeywordIndex::open(&dir.path().join("index.sqlite3")).unwrap();
        // The watcher stored both under (photos, subfolders on).
        let rows: Vec<_> = [&top, &deep]
            .iter()
            .map(|p| {
                (
                    key(p),
                    CachedKeywords {
                        stamp: Fingerprint::capture(p).unwrap(),
                        tags: BTreeSet::from(["moss".to_string()]),
                    },
                )
            })
            .collect();
        index.store(&root, true, &rows).unwrap();
        // The viewer opens the folder without subfolders and still finds them.
        let found = index.load_any(&root, false).unwrap();
        assert!(found.contains_key(&key(&top)) && found.contains_key(&key(&deep)));
        assert!(index.load(&root, false).unwrap().is_empty());
        // A sibling folder with the same name prefix is not included.
        assert!(index
            .load_any(&dir.path().join("photo"), false)
            .unwrap()
            .is_empty());
        let stamp = Fingerprint::capture(&top).unwrap();
        assert!(index.get_any(&top, &stamp).unwrap().is_some());
        std::fs::write(&top, b"changed").unwrap();
        let stamp = Fingerprint::capture(&top).unwrap();
        assert!(index.get_any(&top, &stamp).unwrap().is_none());
    }
    #[test]
    fn same_size_edits_and_sidecar_changes_invalidate_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("image.png");
        fs::write(&image, b"before").unwrap();
        let first = Fingerprint::capture(&image).unwrap();
        fs::write(&image, b"edited").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&image)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1234))
            .unwrap();
        let second = Fingerprint::capture(&image).unwrap();
        assert_ne!(first, second);
        fs::write(crate::metadata::sidecar(&image), b"tags").unwrap();
        let third = Fingerprint::capture(&image).unwrap();
        assert_ne!(second, third);
        fs::remove_file(crate::metadata::sidecar(&image)).unwrap();
        assert_eq!(Fingerprint::capture(&image).unwrap(), second);
    }
}

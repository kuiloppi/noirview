//! A rebuildable, persistent cache. Original image metadata remains authoritative.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::Path,
    time::UNIX_EPOCH,
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
/// The metadata reader revision: raised when a reader learns another keyword
/// field, so unchanged images are read again (also the revision an old
/// `keyword-index.sqlite3` must have to be imported).
pub const READER_REVISION: i64 = 3;
/// The keyword cache. Since 0.1.79 its rows live in the catalog
/// (`catalog.sqlite3` next to the given path; one row per file, whichever
/// folder scope stored it). An old `keyword-index.sqlite3` at the given path is
/// imported once, read-only, and otherwise left alone.
pub struct KeywordIndex {
    catalog: std::cell::RefCell<crate::catalog::Catalog>,
    /// Rows gathered for one larger write (a hard disk rewrites every touched
    /// index page per commit); stored before any read and when the pass ends.
    pending: std::cell::RefCell<crate::catalog::Pending>,
}
fn depth(recursive: bool) -> crate::catalog::Depth {
    if recursive {
        crate::catalog::Depth::Subtree
    } else {
        crate::catalog::Depth::Children
    }
}
impl Fingerprint {
    fn to_entry(&self, tags: BTreeSet<String>) -> crate::catalog::Entry {
        let stamp = |s: &FileStamp| crate::catalog::Stamp {
            size: s.len,
            modified_ns: s.modified.as_deref().and_then(|m| m.parse::<i64>().ok()),
        };
        crate::catalog::Entry {
            image: stamp(&self.image),
            sidecar: self.sidecar.as_ref().map(stamp),
            tags,
        }
    }
    fn from_entry(entry: &crate::catalog::Entry) -> Self {
        let stamp = |s: &crate::catalog::Stamp| FileStamp {
            len: s.size,
            modified: s.modified_ns.map(|ns| ns.to_string()),
        };
        Self {
            image: stamp(&entry.image),
            sidecar: entry.sidecar.as_ref().map(stamp),
        }
    }
}
fn cached(entry: crate::catalog::Entry) -> CachedKeywords {
    CachedKeywords {
        stamp: Fingerprint::from_entry(&entry),
        tags: entry.tags,
    }
}
impl KeywordIndex {
    /// The catalog beside an old keyword index path.
    pub fn catalog_path(path: &Path) -> std::path::PathBuf {
        path.with_file_name("catalog.sqlite3")
    }
    pub fn open(path: &Path) -> Result<Self> {
        let mut catalog = crate::catalog::Catalog::open(&Self::catalog_path(path))
            .context("Cannot open keyword index")?;
        catalog.set_reader_revision(READER_REVISION)?;
        // Upgrade: the old cache's rows once, so no image is read again. A
        // missing or unreadable old file just means a cold first pass.
        if catalog.fact("keyword_index_imported")?.is_none() {
            if path.exists() {
                let _ = catalog.import_keyword_index(path, READER_REVISION);
            }
            catalog.set_fact("keyword_index_imported", "1")?;
        }
        Ok(Self {
            catalog: std::cell::RefCell::new(catalog),
            pending: Default::default(),
        })
    }
    /// Store gathered rows now (callers that batch on their own, such as the
    /// folder watcher, so other readers see them).
    pub fn write_pending(&self) -> Result<()> {
        let mut catalog = self.catalog.borrow_mut();
        self.pending.borrow_mut().flush(&mut catalog)?;
        Ok(())
    }
    /// End of an indexing pass: store everything and let the log go into the
    /// database file (a pause, so the disk wait does not hold up work).
    pub fn finish(&self) -> Result<()> {
        self.write_pending()?;
        self.catalog.borrow().checkpoint()
    }
    pub fn load(&self, folder: &Path, recursive: bool) -> Result<HashMap<Vec<u8>, CachedKeywords>> {
        self.write_pending()?;
        Ok(self
            .catalog
            .borrow()
            .load(folder, depth(recursive))?
            .into_iter()
            .map(|(path, entry)| (path, cached(entry)))
            .collect())
    }
    /// Rows for every file under `folder`, subfolders included whatever
    /// `recursive` says (as before; the caller looks rows up by path). A file
    /// has one row whichever scope stored it.
    pub fn load_any(
        &self,
        folder: &Path,
        _recursive: bool,
    ) -> Result<HashMap<Vec<u8>, CachedKeywords>> {
        self.load(folder, true)
    }
    /// The row for one file if it was read at this file state.
    pub fn get_any(&self, path: &Path, stamp: &Fingerprint) -> Result<Option<CachedKeywords>> {
        self.write_pending()?;
        Ok(self
            .catalog
            .borrow()
            .get(path)?
            .map(cached)
            .filter(|row| &row.stamp == stamp))
    }
    /// Gathers rows (the scope arguments are kept for the callers; a file has
    /// one row). They are written in larger groups, at the latest before the
    /// next read or `finish`.
    pub fn store(
        &mut self,
        _folder: &Path,
        _recursive: bool,
        rows: &[(Vec<u8>, CachedKeywords)],
    ) -> Result<()> {
        let now = std::time::Instant::now();
        let due = {
            let mut pending = self.pending.borrow_mut();
            for (path, row) in rows {
                pending.push(
                    crate::catalog::path_of(path.clone()),
                    row.stamp.to_entry(row.tags.clone()),
                    now,
                );
            }
            pending.due(now)
        };
        if due {
            self.write_pending()?;
        }
        Ok(())
    }
    /// Compare the folder's images and shown keywords with the stored rows
    /// (read only).
    pub fn check(
        &self,
        folder: &Path,
        recursive: bool,
        files: &[(std::path::PathBuf, BTreeSet<String>)],
    ) -> Result<crate::catalog::Check> {
        self.write_pending()?;
        let rows = self.catalog.borrow().tag_rows(folder, depth(recursive))?;
        Ok(crate::catalog::Check::compare(files, rows))
    }
    pub fn prune(
        &mut self,
        folder: &Path,
        recursive: bool,
        valid: &HashSet<Vec<u8>>,
    ) -> Result<()> {
        self.write_pending()?;
        self.catalog
            .borrow_mut()
            .retain(folder, depth(recursive), valid)?;
        Ok(())
    }
    pub fn remove(&self, _folder: &Path, _recursive: bool, path: &Path) -> Result<()> {
        self.write_pending()?;
        self.catalog.borrow_mut().remove(path)?;
        Ok(())
    }
    pub fn clear(&self, folder: &Path, recursive: bool) -> Result<()> {
        self.write_pending()?;
        self.catalog.borrow_mut().clear(folder, depth(recursive))
    }
}
impl Drop for KeywordIndex {
    fn drop(&mut self) {
        let _ = self.write_pending();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;
    fn row(path: &Path, tags: &[&str]) -> (Vec<u8>, CachedKeywords) {
        (
            key(path),
            CachedKeywords {
                stamp: Fingerprint::capture(path).unwrap(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
            },
        )
    }
    #[test]
    fn reader_upgrade_marks_rows_unread_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("keyword-index.sqlite3");
        let image = dir.path().join("old.jpg");
        fs::write(&image, b"image").unwrap();
        let rows = [row(&image, &[])];
        {
            let mut index = KeywordIndex::open(&db).unwrap();
            index.store(dir.path(), false, &rows).unwrap();
            // As if an older reader had stored them.
            index.write_pending().unwrap();
            index.catalog.borrow_mut().set_reader_revision(2).unwrap();
        }
        let mut upgraded = KeywordIndex::open(&db).unwrap();
        assert!(upgraded.load(dir.path(), false).unwrap().is_empty());
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
    fn cache_survives_restart_and_one_row_serves_every_scope() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("photos");
        fs::create_dir_all(&folder).unwrap();
        let image = folder.join("night_日本語.png");
        fs::write(&image, b"pixels").unwrap();
        let stamp = Fingerprint::capture(&image).unwrap();
        let tags = BTreeSet::from(["night & city".into(), "日本語".into()]);
        let db = dir.path().join("keyword-index.sqlite3");
        {
            let mut index = KeywordIndex::open(&db).unwrap();
            index
                .store(
                    &folder,
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
        // The same row with and without subfolders, not for a sibling folder.
        for recursive in [false, true] {
            let rows = index.load(&folder, recursive).unwrap();
            assert_eq!(rows[&key(&image)].tags, tags);
            assert_eq!(rows[&key(&image)].stamp, stamp);
        }
        assert!(index
            .load(&dir.path().join("photo"), true)
            .unwrap()
            .is_empty());
        assert!(index
            .load(&dir.path().join("photos2"), false)
            .unwrap()
            .is_empty());
    }
    /// A keyword index as NoirView before 0.1.79 wrote it.
    fn old_index(db: &Path, rows: &[(&Path, bool, &Path, &Fingerprint, &[&str])]) {
        let c = rusqlite::Connection::open(db).unwrap();
        c.execute_batch(
            "CREATE TABLE keyword_index (folder BLOB NOT NULL, recursive INTEGER NOT NULL,
                path BLOB NOT NULL, stamp TEXT NOT NULL, tags TEXT NOT NULL,
                PRIMARY KEY(folder,recursive,path)) WITHOUT ROWID;
            CREATE TABLE reader_revision (revision INTEGER NOT NULL);
            INSERT INTO reader_revision VALUES(3); PRAGMA user_version=1;",
        )
        .unwrap();
        for (folder, recursive, path, stamp, tags) in rows {
            let tags: BTreeSet<&str> = tags.iter().copied().collect();
            c.execute(
                "INSERT INTO keyword_index VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    key(folder),
                    recursive,
                    key(path),
                    serde_json::to_string(stamp).unwrap(),
                    serde_json::to_string(&tags).unwrap()
                ],
            )
            .unwrap();
        }
    }
    #[test]
    fn an_old_keyword_index_is_imported_once_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        fs::create_dir_all(root.join("sub")).unwrap();
        let (top, deep) = (root.join("a.jpg"), root.join("sub/b.jpg"));
        fs::write(&top, b"x").unwrap();
        fs::write(&deep, b"y").unwrap();
        fs::write(crate::metadata::sidecar(&deep), b"<x/>").unwrap();
        let (st, sd) = (
            Fingerprint::capture(&top).unwrap(),
            Fingerprint::capture(&deep).unwrap(),
        );
        let db = dir.path().join("keyword-index.sqlite3");
        // Rows under two scopes, one path under both.
        old_index(
            &db,
            &[
                (&root, false, &top, &st, &["moss"]),
                (&root, true, &top, &st, &["moss"]),
                (&root, true, &deep, &sd, &["fern"]),
            ],
        );
        let before = fs::read(&db).unwrap();
        {
            let index = KeywordIndex::open(&db).unwrap();
            let rows = index.load(&root, true).unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[&key(&top)].stamp, st);
            assert_eq!(rows[&key(&deep)].stamp, sd);
            assert!(rows[&key(&deep)].tags.contains("fern"));
            // Warm: the files' own fingerprints find them.
            assert!(index.get_any(&deep, &sd).unwrap().is_some());
        }
        assert_eq!(
            fs::read(&db).unwrap(),
            before,
            "the old file is not changed"
        );
        // Not imported again: a row added to the old file later stays there.
        let late = root.join("c.jpg");
        fs::write(&late, b"z").unwrap();
        let sl = Fingerprint::capture(&late).unwrap();
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute(
                "INSERT INTO keyword_index VALUES(?1,0,?2,?3,'[]')",
                rusqlite::params![key(&root), key(&late), serde_json::to_string(&sl).unwrap()],
            )
            .unwrap();
        let index = KeywordIndex::open(&db).unwrap();
        assert!(!index.load(&root, false).unwrap().contains_key(&key(&late)));
    }
    #[test]
    fn a_corrupt_old_index_gives_an_empty_working_cache() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("keyword-index.sqlite3");
        fs::write(&db, b"this is not a database").unwrap();
        let image = dir.path().join("a.jpg");
        fs::write(&image, b"x").unwrap();
        let mut index = KeywordIndex::open(&db).unwrap();
        assert!(index.load(dir.path(), false).unwrap().is_empty());
        index
            .store(dir.path(), false, &[row(&image, &["a"])])
            .unwrap();
        assert_eq!(index.load(dir.path(), false).unwrap().len(), 1);
    }
    #[test]
    fn fingerprints_round_trip_through_the_catalog() {
        let stamps = [
            Fingerprint {
                image: FileStamp::new(
                    10,
                    Some(UNIX_EPOCH + std::time::Duration::from_nanos(1_700_000_000_123_456_789)),
                ),
                sidecar: Some(FileStamp::new(
                    3,
                    Some(UNIX_EPOCH - std::time::Duration::from_secs(5)),
                )),
            },
            Fingerprint {
                image: FileStamp::new(0, None),
                sidecar: None,
            },
        ];
        for stamp in stamps {
            let back = Fingerprint::from_entry(&stamp.to_entry(BTreeSet::new()));
            assert_eq!(back, stamp);
            assert_eq!(back.reusable(), stamp.reusable());
        }
        // A time beyond 64-bit nanoseconds is kept as unknown: never reused.
        let far = Fingerprint {
            image: FileStamp {
                len: 1,
                modified: Some("99999999999999999999999".into()),
            },
            sidecar: None,
        };
        assert!(!Fingerprint::from_entry(&far.to_entry(BTreeSet::new())).reusable());
    }
    #[test]
    fn stored_rows_are_seen_by_the_next_read_before_they_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("a.jpg");
        fs::write(&image, b"x").unwrap();
        let mut index = KeywordIndex::open(&dir.path().join("keyword-index.sqlite3")).unwrap();
        index
            .store(dir.path(), true, &[row(&image, &["a"])])
            .unwrap();
        assert!(
            !index.pending.borrow().is_empty(),
            "gathered, not yet written"
        );
        let stamp = Fingerprint::capture(&image).unwrap();
        assert!(index.get_any(&image, &stamp).unwrap().is_some());
        index.remove(dir.path(), true, &image).unwrap();
        assert!(index.get_any(&image, &stamp).unwrap().is_none());
        index
            .store(dir.path(), true, &[row(&image, &["a"])])
            .unwrap();
        index.finish().unwrap();
        assert!(index.pending.borrow().is_empty());
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
        // One row per file: the folder's own images also without subfolders.
        let children = index.load(&root, false).unwrap();
        assert!(children.contains_key(&key(&top)) && !children.contains_key(&key(&deep)));
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
            .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1234))
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

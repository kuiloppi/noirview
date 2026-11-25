//! Rebuildable catalog of folders and images. The keyword cache stores its rows
//! here (`keyword_index.rs`).
//!
//! One row per folder and one per image, with integer ids. An image's path is its
//! folder's path plus its file name, so the folder prefix is stored once. Keywords
//! are kept twice inside this one database and one transaction: as text on the
//! image row for fast loading, and as a relation for keyword queries.
//!
//! Everything here can be rebuilt from the image files. Marks and settings stay in
//! the durable user database (`state.rs`). Ids are stable while the file exists
//! but are not a durable identity: a rebuilt catalog assigns new ones.
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf, MAIN_SEPARATOR},
    time::{Duration, UNIX_EPOCH},
};

#[cfg(test)]
mod scale_bench;

/// Size and modification time of one file. `modified_ns` is `None` when the time
/// is unavailable or outside the signed 64-bit nanosecond range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub modified_ns: Option<i64>,
}
impl Stamp {
    pub fn from_metadata(metadata: &fs::Metadata) -> Self {
        let modified_ns =
            metadata
                .modified()
                .ok()
                .and_then(|time| match time.duration_since(UNIX_EPOCH) {
                    Ok(after) => i64::try_from(after.as_nanos()).ok(),
                    Err(before) => i64::try_from(before.duration().as_nanos())
                        .ok()
                        .map(|ns| -ns),
                });
        Self {
            size: metadata.len(),
            modified_ns,
        }
    }
}
/// What is known about one image: the file state its keywords were read at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub image: Stamp,
    pub sidecar: Option<Stamp>,
    pub tags: BTreeSet<String>,
}
impl Entry {
    /// Both timestamps are known, so equality with a fresh stamp proves "unchanged".
    pub fn reusable(&self) -> bool {
        self.image.modified_ns.is_some()
            && self.sidecar.is_none_or(|side| side.modified_ns.is_some())
    }
}
/// Images grouped by folder: the folder's path bytes and, for each image, its id
/// and file-name bytes.
pub type Listing = Vec<(Vec<u8>, Vec<(u32, Vec<u8>)>)>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// Images directly inside the folder.
    Children,
    /// Images in the folder and in every folder below it.
    Subtree,
}

/// Full-path bytes in the platform encoding, identical to `keyword_index::key`.
pub fn key(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}
/// The inverse of `key` for bytes this module stored.
pub fn path_of(bytes: Vec<u8>) -> PathBuf {
    // SAFETY: every stored value was produced by `as_encoded_bytes` on this
    // platform, or by joining two such values with the ASCII separator.
    unsafe { std::ffi::OsString::from_encoded_bytes_unchecked(bytes) }.into()
}
fn split(path: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let folder = path.parent().context("Image path has no folder")?;
    let name = path.file_name().context("Image path has no file name")?;
    Ok((key(folder), name.as_encoded_bytes().to_vec()))
}
pub(crate) fn join(folder: &[u8], name: &[u8]) -> Vec<u8> {
    let mut full = Vec::with_capacity(folder.len() + 1 + name.len());
    full.extend_from_slice(folder);
    if folder.last() != Some(&(MAIN_SEPARATOR as u8)) {
        full.push(MAIN_SEPARATOR as u8);
    }
    full.extend_from_slice(name);
    full
}
/// Half-open byte range that contains exactly the paths below `folder`.
fn below(folder: &Path) -> (Vec<u8>, Vec<u8>) {
    let mut low = key(folder);
    if low.last() != Some(&(MAIN_SEPARATOR as u8)) {
        low.push(MAIN_SEPARATOR as u8);
    }
    let mut high = low.clone();
    *high.last_mut().unwrap() += 1;
    (low, high)
}
fn encode_tags(tags: &BTreeSet<String>) -> Result<String> {
    Ok(serde_json::to_string(tags)?)
}
fn decode_tags(text: &str) -> Result<BTreeSet<String>> {
    if text == "[]" {
        return Ok(BTreeSet::new());
    }
    Ok(serde_json::from_str(text)?)
}
fn as_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

const SCHEMA: i64 = 1;
/// Pages written to the write-ahead log before SQLite checkpoints on its own.
/// Every checkpoint waits for the disk; with the default of 1,000 pages a batch
/// of 64 images triggers one almost every time, which a hard disk answers in
/// about a tenth of a second (measured: 100,000 images took 200 s on an HDD
/// against 6 s on an SSD). 16,384 pages are 64 MiB of log.
const CHECKPOINT_PAGES: i64 = 16_384;
/// The log file is cut back to this size after a checkpoint.
const LOG_LIMIT_BYTES: i64 = 128 * 1024 * 1024;

pub struct Catalog {
    connection: Connection,
    /// Keyword ids are stable while a keyword has rows; cleared when keywords are deleted.
    keyword_ids: HashMap<String, i64>,
}
impl Catalog {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path).context("Cannot open the catalog")?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(
            version <= SCHEMA,
            "Catalog belongs to a newer NoirView version"
        );
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS folders(
                id INTEGER PRIMARY KEY,
                path BLOB NOT NULL UNIQUE);
            CREATE TABLE IF NOT EXISTS images(
                id INTEGER PRIMARY KEY,
                folder INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                name BLOB NOT NULL,
                size INTEGER NOT NULL,
                modified INTEGER,
                sidecar_size INTEGER,
                sidecar_modified INTEGER,
                identity TEXT,
                keywords_current INTEGER NOT NULL DEFAULT 1,
                tags TEXT NOT NULL,
                UNIQUE(folder,name));
            CREATE INDEX IF NOT EXISTS images_by_identity ON images(identity)
                WHERE identity IS NOT NULL;
            CREATE TABLE IF NOT EXISTS keywords(
                id INTEGER PRIMARY KEY,
                keyword TEXT NOT NULL UNIQUE);
            CREATE TABLE IF NOT EXISTS image_keywords(
                image INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
                keyword INTEGER NOT NULL REFERENCES keywords(id),
                PRIMARY KEY(image,keyword)) WITHOUT ROWID;
            CREATE INDEX IF NOT EXISTS image_keywords_by_keyword
                ON image_keywords(keyword,image);
            CREATE TABLE IF NOT EXISTS facts(name TEXT PRIMARY KEY, value TEXT NOT NULL)
                WITHOUT ROWID;",
        )?;
        connection.pragma_update(None, "user_version", SCHEMA)?;
        connection.pragma_update(None, "wal_autocheckpoint", CHECKPOINT_PAGES)?;
        // This pragma answers with the value it set.
        connection.query_row(
            &format!("PRAGMA journal_size_limit={LOG_LIMIT_BYTES}"),
            [],
            |_| Ok(()),
        )?;
        Ok(Self {
            connection,
            keyword_ids: HashMap::new(),
        })
    }
    /// A stored fact (small values kept with the catalog).
    pub fn fact(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row("SELECT value FROM facts WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn set_fact(&mut self, name: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO facts(name,value) VALUES(?1,?2)
            ON CONFLICT(name) DO UPDATE SET value=excluded.value",
            [name, value],
        )?;
        Ok(())
    }
    /// Record the metadata-reader revision. When it differs from the stored one,
    /// every image keeps its row and id but its keywords count as unread.
    pub fn set_reader_revision(&mut self, revision: i64) -> Result<()> {
        let tx = self.connection.transaction()?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT value FROM facts WHERE name='reader_revision'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if stored.as_deref() != Some(revision.to_string().as_str()) {
            tx.execute("UPDATE images SET keywords_current=0", [])?;
            tx.execute(
                "INSERT INTO facts(name,value) VALUES('reader_revision',?1)
                ON CONFLICT(name) DO UPDATE SET value=excluded.value",
                [revision.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Insert or update images with the keywords read at the given file state.
    /// An existing image keeps its id.
    pub fn store(&mut self, rows: &[(PathBuf, Entry)]) -> Result<()> {
        let tx = self.connection.transaction()?;
        {
            let mut folders: HashMap<Vec<u8>, i64> = HashMap::new();
            let mut add_folder = tx.prepare_cached(
                "INSERT INTO folders(path) VALUES(?1) ON CONFLICT(path) DO NOTHING",
            )?;
            let mut folder_id = tx.prepare_cached("SELECT id FROM folders WHERE path=?1")?;
            let mut upsert = tx.prepare_cached(
                "INSERT INTO images(folder,name,size,modified,sidecar_size,sidecar_modified,keywords_current,tags)
                VALUES(?1,?2,?3,?4,?5,?6,1,?7)
                ON CONFLICT(folder,name) DO UPDATE SET size=excluded.size,modified=excluded.modified,
                    sidecar_size=excluded.sidecar_size,sidecar_modified=excluded.sidecar_modified,
                    keywords_current=1,tags=excluded.tags
                RETURNING id",
            )?;
            let mut unlink = tx.prepare_cached("DELETE FROM image_keywords WHERE image=?1")?;
            let mut add_keyword = tx.prepare_cached(
                "INSERT INTO keywords(keyword) VALUES(?1) ON CONFLICT(keyword) DO NOTHING",
            )?;
            let mut keyword_id = tx.prepare_cached("SELECT id FROM keywords WHERE keyword=?1")?;
            let mut link = tx.prepare_cached(
                "INSERT INTO image_keywords(image,keyword) VALUES(?1,?2) ON CONFLICT DO NOTHING",
            )?;
            for (path, entry) in rows {
                let (folder, name) = split(path)?;
                let folder = match folders.get(&folder) {
                    Some(id) => *id,
                    None => {
                        add_folder.execute([&folder])?;
                        let id: i64 = folder_id.query_row([&folder], |r| r.get(0))?;
                        folders.insert(folder, id);
                        id
                    }
                };
                let image: i64 = upsert.query_row(
                    params![
                        folder,
                        name,
                        as_i64(entry.image.size),
                        entry.image.modified_ns,
                        entry.sidecar.map(|side| as_i64(side.size)),
                        entry.sidecar.and_then(|side| side.modified_ns),
                        encode_tags(&entry.tags)?
                    ],
                    |r| r.get(0),
                )?;
                unlink.execute([image])?;
                for tag in &entry.tags {
                    let keyword = match self.keyword_ids.get(tag) {
                        Some(id) => *id,
                        None => {
                            add_keyword.execute([tag])?;
                            let id: i64 = keyword_id.query_row([tag], |r| r.get(0))?;
                            self.keyword_ids.insert(tag.clone(), id);
                            id
                        }
                    };
                    link.execute(params![image, keyword])?;
                }
            }
        }
        if let Err(error) = tx.commit() {
            // Ids cached during a rolled-back transaction may not exist.
            self.keyword_ids.clear();
            return Err(error.into());
        }
        Ok(())
    }
    /// Images whose keywords are current, keyed by full-path bytes.
    pub fn load(&self, folder: &Path, depth: Depth) -> Result<HashMap<Vec<u8>, Entry>> {
        let mut result = HashMap::new();
        self.scan(folder, depth, true, |_, path, entry| {
            result.insert(path, entry);
        })?;
        Ok(result)
    }
    /// Every image of the scope with its id, including images whose keywords are
    /// not current. Order is unspecified.
    pub fn ids(&self, folder: &Path, depth: Depth) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut result = Vec::new();
        self.scan(folder, depth, false, |id, path, _| {
            result.push((id, path));
        })?;
        Ok(result)
    }
    /// Every image of the scope grouped by folder. Order is unspecified.
    pub fn listing(&self, folder: &Path, depth: Depth) -> Result<Listing> {
        let mut result = Vec::new();
        for (folder_id, folder_path) in self.folders(folder, depth)? {
            let mut query = self
                .connection
                .prepare_cached("SELECT id,name FROM images WHERE folder=?1")?;
            let mut rows = query.query([folder_id])?;
            let mut images = Vec::new();
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                images.push((
                    u32::try_from(id).context("Catalog id out of range; rebuild the catalog")?,
                    row.get(1)?,
                ));
            }
            result.push((folder_path, images));
        }
        Ok(result)
    }
    fn scan(
        &self,
        folder: &Path,
        depth: Depth,
        current_only: bool,
        mut each: impl FnMut(u32, Vec<u8>, Entry),
    ) -> Result<()> {
        for (folder_id, folder_path) in self.folders(folder, depth)? {
            let mut query = self.connection.prepare_cached(
                "SELECT id,name,size,modified,sidecar_size,sidecar_modified,tags,keywords_current
                FROM images WHERE folder=?1",
            )?;
            let mut rows = query.query([folder_id])?;
            while let Some(row) = rows.next()? {
                if current_only && !row.get::<_, bool>(7)? {
                    continue;
                }
                let id: i64 = row.get(0)?;
                let name: Vec<u8> = row.get(1)?;
                let sidecar_size: Option<i64> = row.get(4)?;
                each(
                    u32::try_from(id).context("Catalog id out of range; rebuild the catalog")?,
                    join(&folder_path, &name),
                    Entry {
                        image: Stamp {
                            size: row.get::<_, i64>(2)? as u64,
                            modified_ns: row.get(3)?,
                        },
                        sidecar: match sidecar_size {
                            Some(size) => Some(Stamp {
                                size: size as u64,
                                modified_ns: row.get(5)?,
                            }),
                            None => None,
                        },
                        tags: decode_tags(&row.get::<_, String>(6)?)?,
                    },
                );
            }
        }
        Ok(())
    }
    fn folders(&self, folder: &Path, depth: Depth) -> Result<Vec<(i64, Vec<u8>)>> {
        let mut found = Vec::new();
        let exact = key(folder);
        if let Some(id) = self
            .connection
            .prepare_cached("SELECT id FROM folders WHERE path=?1")?
            .query_row([&exact], |r| r.get::<_, i64>(0))
            .optional()?
        {
            found.push((id, exact));
        }
        if depth == Depth::Subtree {
            let (low, high) = below(folder);
            let mut query = self
                .connection
                .prepare_cached("SELECT id,path FROM folders WHERE path>=?1 AND path<?2")?;
            let mut rows = query.query(params![low, high])?;
            while let Some(row) = rows.next()? {
                found.push((row.get(0)?, row.get(1)?));
            }
        }
        Ok(found)
    }
    /// For a consistency check: every image in the scope with current
    /// keywords, its `tags` text and its keyword relation (read only).
    pub fn tag_rows(&self, folder: &Path, depth: Depth) -> Result<TagRows> {
        let mut found = HashMap::new();
        for (id, path) in self.folders(folder, depth)? {
            let mut names = HashMap::new();
            let mut images = self.connection.prepare_cached(
                "SELECT id,name,tags FROM images WHERE folder=?1 AND keywords_current=1",
            )?;
            let mut rows = images.query([id])?;
            while let Some(row) = rows.next()? {
                let full = join(&path, &row.get::<_, Vec<u8>>(1)?);
                names.insert(row.get::<_, i64>(0)?, full.clone());
                found.insert(
                    full,
                    (decode_tags(&row.get::<_, String>(2)?)?, BTreeSet::new()),
                );
            }
            let mut relation = self.connection.prepare_cached(
                "SELECT ik.image,k.keyword FROM image_keywords ik
                JOIN keywords k ON k.id=ik.keyword JOIN images i ON i.id=ik.image
                WHERE i.folder=?1 AND i.keywords_current=1",
            )?;
            let mut rows = relation.query([id])?;
            while let Some(row) = rows.next()? {
                if let Some(entry) = names
                    .get(&row.get::<_, i64>(0)?)
                    .and_then(|full| found.get_mut(full))
                {
                    entry.1.insert(row.get(1)?);
                }
            }
        }
        Ok(found)
    }
    /// One image with current keywords, or `None`.
    pub fn get(&self, path: &Path) -> Result<Option<Entry>> {
        let (folder, name) = split(path)?;
        let row = self
            .connection
            .prepare_cached(
                "SELECT i.size,i.modified,i.sidecar_size,i.sidecar_modified,i.tags
                FROM images i JOIN folders f ON f.id=i.folder
                WHERE f.path=?1 AND i.name=?2 AND i.keywords_current=1",
            )?
            .query_row(params![folder, name], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .optional()?;
        row.map(
            |(size, modified_ns, sidecar_size, sidecar_modified, tags)| {
                Ok(Entry {
                    image: Stamp {
                        size: size as u64,
                        modified_ns,
                    },
                    sidecar: sidecar_size.map(|size| Stamp {
                        size: size as u64,
                        modified_ns: sidecar_modified,
                    }),
                    tags: decode_tags(&tags)?,
                })
            },
        )
        .transpose()
    }
    /// Delete the scope's images that are not in `valid` (full-path bytes) and
    /// folders left without images. Returns the number of images deleted.
    pub fn retain(
        &mut self,
        folder: &Path,
        depth: Depth,
        valid: &HashSet<Vec<u8>>,
    ) -> Result<usize> {
        let stale: Vec<u32> = self
            .ids(folder, depth)?
            .into_iter()
            .filter(|(_, path)| !valid.contains(path))
            .map(|(id, _)| id)
            .collect();
        let scope = self.folders(folder, depth)?;
        let tx = self.connection.transaction()?;
        {
            let mut delete = tx.prepare_cached("DELETE FROM images WHERE id=?1")?;
            for id in &stale {
                delete.execute([id])?;
            }
            let mut empty = tx.prepare_cached(
                "DELETE FROM folders WHERE id=?1
                AND NOT EXISTS(SELECT 1 FROM images WHERE folder=?1)",
            )?;
            for (id, _) in &scope {
                empty.execute([id])?;
            }
        }
        tx.commit()?;
        Ok(stale.len())
    }
    pub fn remove(&mut self, path: &Path) -> Result<bool> {
        let (folder, name) = split(path)?;
        Ok(self.connection.execute(
            "DELETE FROM images WHERE name=?2 AND folder=(SELECT id FROM folders WHERE path=?1)",
            params![folder, name],
        )? > 0)
    }
    /// Forget everything in the scope.
    pub fn clear(&mut self, folder: &Path, depth: Depth) -> Result<()> {
        self.retain(folder, depth, &HashSet::new()).map(|_| ())
    }
    /// Move the write-ahead log into the database file and wait for the disk.
    /// Call at a natural pause, such as the end of an indexing pass; closing the
    /// last connection does the same.
    pub fn checkpoint(&self) -> Result<()> {
        self.connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    }
    /// Remove keywords no image uses. Call occasionally, not per batch.
    pub fn drop_unused_keywords(&mut self) -> Result<usize> {
        self.keyword_ids.clear();
        Ok(self.connection.execute(
            "DELETE FROM keywords WHERE NOT EXISTS
                (SELECT 1 FROM image_keywords WHERE keyword=keywords.id)",
            [],
        )?)
    }
    pub fn set_identity(&mut self, path: &Path, identity: Option<&str>) -> Result<bool> {
        let (folder, name) = split(path)?;
        Ok(self.connection.execute(
            "UPDATE images SET identity=?3
            WHERE name=?2 AND folder=(SELECT id FROM folders WHERE path=?1)",
            params![folder, name, identity],
        )? > 0)
    }
    /// Ids of all images carrying the keyword, ascending.
    pub fn ids_with_keyword(&self, keyword: &str) -> Result<Vec<u32>> {
        let mut query = self.connection.prepare_cached(
            "SELECT ik.image FROM image_keywords ik JOIN keywords k ON k.id=ik.keyword
            WHERE k.keyword=?1 ORDER BY ik.image",
        )?;
        let rows = query.query_map([keyword], |r| r.get::<_, i64>(0))?;
        rows.map(|id| u32::try_from(id?).context("Catalog id out of range"))
            .collect()
    }
    /// Copy rows from a version-1 `keyword-index.sqlite3` whose reader revision is
    /// `reader_revision`. The source is opened read-only and never changed. When
    /// several scopes hold the same path, a row with known timestamps wins.
    pub fn import_keyword_index(&mut self, source: &Path, reader_revision: i64) -> Result<usize> {
        #[derive(serde::Deserialize)]
        struct OldStamp {
            len: u64,
            modified: Option<String>,
        }
        #[derive(serde::Deserialize)]
        struct OldFingerprint {
            image: OldStamp,
            sidecar: Option<OldStamp>,
        }
        fn stamp(old: OldStamp) -> Stamp {
            Stamp {
                size: old.len,
                modified_ns: old.modified.and_then(|text| text.parse().ok()),
            }
        }
        let old = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let revision: Option<i64> = old
            .query_row("SELECT revision FROM reader_revision LIMIT 1", [], |r| {
                r.get(0)
            })
            .optional()
            .unwrap_or(None);
        if revision != Some(reader_revision) {
            return Ok(0);
        }
        let mut merged: HashMap<Vec<u8>, Entry> = HashMap::new();
        {
            let mut query = old.prepare("SELECT path,stamp,tags FROM keyword_index")?;
            let mut rows = query.query([])?;
            while let Some(row) = rows.next()? {
                let path: Vec<u8> = row.get(0)?;
                let Ok(fingerprint) =
                    serde_json::from_str::<OldFingerprint>(&row.get::<_, String>(1)?)
                else {
                    continue;
                };
                let Ok(tags) = decode_tags(&row.get::<_, String>(2)?) else {
                    continue;
                };
                let entry = Entry {
                    image: stamp(fingerprint.image),
                    sidecar: fingerprint.sidecar.map(stamp),
                    tags,
                };
                if entry.reusable() || !merged.contains_key(&path) {
                    merged.insert(path, entry);
                }
            }
        }
        let rows: Vec<_> = merged
            .into_iter()
            .map(|(path, entry)| (path_of(path), entry))
            .collect();
        for batch in rows.chunks(1024) {
            self.store(batch)?;
        }
        Ok(rows.len())
    }
}

/// Per image (full-path bytes): its keyword text and its keyword relation.
pub type TagRows = HashMap<Vec<u8>, (BTreeSet<String>, BTreeSet<String>)>;
/// What a consistency check found (`Settings → Folders & tags`).
#[derive(Debug, Default, PartialEq)]
pub struct Check {
    pub images: usize,
    /// Keywords shown in NoirView differ from the catalog's.
    pub shown_differs: Vec<(PathBuf, BTreeSet<String>, BTreeSet<String>)>,
    /// The catalog's keyword text and keyword relation disagree.
    pub relation_differs: Vec<PathBuf>,
    /// In the folder's list, not (or not yet) in the catalog.
    pub missing: Vec<PathBuf>,
    /// In the catalog, not in the folder's list.
    pub extra: Vec<PathBuf>,
}
impl Check {
    /// Compares the folder's images and their shown keywords with the
    /// catalog's rows. Changes nothing.
    pub fn compare(files: &[(PathBuf, BTreeSet<String>)], mut rows: TagRows) -> Self {
        let mut check = Self {
            images: files.len(),
            ..Self::default()
        };
        for (path, shown) in files {
            match rows.remove(&key(path)) {
                None => check.missing.push(path.clone()),
                Some((tags, relation)) => {
                    if &tags != shown {
                        check
                            .shown_differs
                            .push((path.clone(), shown.clone(), tags.clone()));
                    }
                    if tags != relation {
                        check.relation_differs.push(path.clone());
                    }
                }
            }
        }
        check.extra = rows.into_keys().map(path_of).collect();
        check.extra.sort();
        check
    }
    pub fn is_clean(&self) -> bool {
        self.shown_differs.is_empty()
            && self.relation_differs.is_empty()
            && self.missing.is_empty()
            && self.extra.is_empty()
    }
    /// A short report with the first few differences of each kind.
    pub fn report(&self) -> String {
        let mut text = format!(
            "{} image(s) checked.\nKeywords shown ≠ catalog: {}\nCatalog keywords ≠ keyword relation: {}\nNot in the catalog: {}\nIn the catalog, not in the folder: {}",
            self.images,
            self.shown_differs.len(),
            self.relation_differs.len(),
            self.missing.len(),
            self.extra.len()
        );
        let name = |p: &PathBuf| p.display().to_string();
        let list = |text: &mut String, title: &str, items: Vec<String>| {
            if !items.is_empty() {
                text.push_str(&format!("\n\n{title}:"));
                for item in items.iter().take(5) {
                    text.push_str(&format!("\n  {item}"));
                }
                if items.len() > 5 {
                    text.push_str(&format!("\n  … and {} more", items.len() - 5));
                }
            }
        };
        let tags = |t: &BTreeSet<String>| t.iter().cloned().collect::<Vec<_>>().join(", ");
        list(
            &mut text,
            "Keywords shown ≠ catalog",
            self.shown_differs
                .iter()
                .map(|(p, shown, tags_)| {
                    format!(
                        "{} — shown [{}], catalog [{}]",
                        name(p),
                        tags(shown),
                        tags(tags_)
                    )
                })
                .collect(),
        );
        list(
            &mut text,
            "Catalog keywords ≠ keyword relation",
            self.relation_differs.iter().map(name).collect(),
        );
        list(
            &mut text,
            "Not in the catalog",
            self.missing.iter().map(name).collect(),
        );
        list(
            &mut text,
            "In the catalog, not in the folder",
            self.extra.iter().map(name).collect(),
        );
        if self.is_clean() {
            text.push_str("\n\nNo differences.");
        }
        text
    }
}

/// Rows waiting to be stored together. Each commit rewrites every index page it
/// touches, and keyword relations of a few images are spread over many pages, so
/// many small commits write far more than the data they add: measured on a hard
/// disk, 100,000 images took 55 s in batches of 64 and 10 s in batches of 1,024.
/// Rows held here are not visible to `Catalog::load` or `get` until flushed, and
/// are lost if the program stops first; they are then simply read again.
#[derive(Default)]
pub struct Pending {
    rows: Vec<(PathBuf, Entry)>,
    oldest: Option<std::time::Instant>,
}
impl Pending {
    pub const ROWS: usize = 1024;
    pub const AGE: Duration = Duration::from_secs(2);

    pub fn push(&mut self, path: PathBuf, entry: Entry, now: std::time::Instant) {
        self.oldest.get_or_insert(now);
        self.rows.push((path, entry));
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    /// Enough rows have gathered, or the oldest has waited long enough.
    pub fn due(&self, now: std::time::Instant) -> bool {
        self.rows.len() >= Self::ROWS
            || self
                .oldest
                .is_some_and(|oldest| now.duration_since(oldest) >= Self::AGE)
    }
    /// Store everything in one transaction. On failure the rows are kept.
    pub fn flush(&mut self, catalog: &mut Catalog) -> Result<usize> {
        if self.rows.is_empty() {
            return Ok(0);
        }
        catalog.store(&self.rows)?;
        self.oldest = None;
        Ok(std::mem::take(&mut self.rows).len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(size: u64, tags: &[&str]) -> Entry {
        Entry {
            image: Stamp {
                size,
                modified_ns: Some(1_700_000_000_000_000_000),
            },
            sidecar: None,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        }
    }
    fn catalog(dir: &Path) -> Catalog {
        Catalog::open(&dir.join("catalog.sqlite3")).unwrap()
    }
    fn paths(found: &[(u32, Vec<u8>)]) -> BTreeSet<PathBuf> {
        found.iter().map(|(_, p)| path_of(p.clone())).collect()
    }
    #[test]
    fn children_and_subtree_scopes_do_not_leak_into_similarly_named_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        let files = [
            root.join("a.jpg"),
            root.join("trip").join("b.jpg"),
            root.join("trip").join("day 1").join("c.jpg"),
            dir.path().join("photos-old").join("d.jpg"),
        ];
        let mut c = catalog(dir.path());
        c.store(
            &files
                .iter()
                .map(|p| (p.clone(), entry(1, &["x"])))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            paths(&c.ids(&root, Depth::Children).unwrap()),
            BTreeSet::from([files[0].clone()])
        );
        assert_eq!(
            paths(&c.ids(&root, Depth::Subtree).unwrap()),
            files[..3].iter().cloned().collect()
        );
        assert_eq!(
            paths(&c.ids(&root.join("trip"), Depth::Subtree).unwrap()),
            files[1..3].iter().cloned().collect()
        );
        assert_eq!(c.load(&root, Depth::Subtree).unwrap().len(), 3);
        assert!(c
            .ids(&dir.path().join("missing"), Depth::Subtree)
            .unwrap()
            .is_empty());
    }
    #[test]
    fn storing_again_keeps_the_id_and_replaces_stamps_and_keyword_relations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("夜 & city").join("kuva ä.png");
        let mut c = catalog(dir.path());
        c.store(&[(path.clone(), entry(10, &["night", "日本語"]))])
            .unwrap();
        let id = c.ids(path.parent().unwrap(), Depth::Children).unwrap()[0].0;
        assert_eq!(c.ids_with_keyword("night").unwrap(), vec![id]);
        let mut changed = entry(11, &["city"]);
        changed.sidecar = Some(Stamp {
            size: 7,
            modified_ns: None,
        });
        c.store(&[(path.clone(), changed.clone())]).unwrap();
        assert_eq!(
            c.ids(path.parent().unwrap(), Depth::Children).unwrap()[0].0,
            id
        );
        assert_eq!(c.get(&path).unwrap(), Some(changed.clone()));
        assert!(!changed.reusable());
        assert!(c.ids_with_keyword("night").unwrap().is_empty());
        assert_eq!(c.ids_with_keyword("city").unwrap(), vec![id]);
        assert_eq!(c.drop_unused_keywords().unwrap(), 2);
        // The keyword-id cache was reset with the deletion.
        c.store(&[(path.clone(), entry(12, &["night"]))]).unwrap();
        assert_eq!(c.ids_with_keyword("night").unwrap(), vec![id]);
    }
    #[test]
    fn retain_remove_and_clear_respect_the_scope() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        let (a, b, deep) = (
            root.join("a.jpg"),
            root.join("b.jpg"),
            root.join("sub").join("c.jpg"),
        );
        let mut c = catalog(dir.path());
        c.store(&[
            (a.clone(), entry(1, &["t"])),
            (b.clone(), entry(1, &["t"])),
            (deep.clone(), entry(1, &["t"])),
        ])
        .unwrap();
        // Non-recursive pruning with only `a` valid must not touch the subfolder.
        assert_eq!(
            c.retain(&root, Depth::Children, &HashSet::from([key(&a)]))
                .unwrap(),
            1
        );
        assert_eq!(
            paths(&c.ids(&root, Depth::Subtree).unwrap()),
            BTreeSet::from([a.clone(), deep.clone()])
        );
        assert_eq!(c.ids_with_keyword("t").unwrap().len(), 2);
        assert!(c.remove(&deep).unwrap());
        assert!(!c.remove(&deep).unwrap());
        c.clear(&root, Depth::Subtree).unwrap();
        assert!(c.ids(&root, Depth::Subtree).unwrap().is_empty());
        assert!(c.ids_with_keyword("t").unwrap().is_empty());
        let folders: i64 = c
            .connection
            .query_row("SELECT count(*) FROM folders", [], |r| r.get(0))
            .unwrap();
        assert_eq!(folders, 0); // clearing a scope also drops its emptied folders
    }
    #[test]
    fn a_new_reader_revision_hides_keywords_but_keeps_rows_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        {
            let mut c = catalog(dir.path());
            c.set_reader_revision(3).unwrap();
            c.store(&[(path.clone(), entry(1, &["t"]))]).unwrap();
            c.set_identity(&path, Some("unix:1:2:3")).unwrap();
            c.set_reader_revision(3).unwrap();
            assert!(c.get(&path).unwrap().is_some());
        }
        let mut c = catalog(dir.path());
        let id = c.ids(dir.path(), Depth::Children).unwrap()[0].0;
        c.set_reader_revision(4).unwrap();
        assert!(c.get(&path).unwrap().is_none());
        assert!(c.load(dir.path(), Depth::Children).unwrap().is_empty());
        assert_eq!(c.ids(dir.path(), Depth::Children).unwrap()[0].0, id);
        c.store(&[(path.clone(), entry(1, &["t"]))]).unwrap();
        assert!(c.get(&path).unwrap().is_some());
        let identity: Option<String> = c
            .connection
            .query_row("SELECT identity FROM images", [], |r| r.get(0))
            .unwrap();
        assert_eq!(identity.as_deref(), Some("unix:1:2:3"));
    }
    #[test]
    fn a_desynchronised_catalog_is_reported_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = catalog(dir.path());
        let folder = dir.path().join("photos");
        let path = |n: &str| folder.join(n);
        c.store(&[
            (path("same.jpg"), entry(1, &["a", "b"])),
            (path("differs.jpg"), entry(2, &["old"])),
            (path("broken.jpg"), entry(3, &["x"])),
            (path("extra.jpg"), entry(4, &[])),
        ])
        .unwrap();
        // The keyword relation of one image no longer matches its text.
        c.connection
            .execute(
                "DELETE FROM image_keywords WHERE image=(SELECT id FROM images WHERE name=?1)",
                [b"broken.jpg".as_slice()],
            )
            .unwrap();
        let set = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        let files = vec![
            (path("same.jpg"), set(&["a", "b"])),
            (path("differs.jpg"), set(&["new"])),
            (path("broken.jpg"), set(&["x"])),
            (path("missing.jpg"), set(&[])),
        ];
        let check = Check::compare(&files, c.tag_rows(&folder, Depth::Children).unwrap());
        assert_eq!(
            check,
            Check {
                images: 4,
                shown_differs: vec![(path("differs.jpg"), set(&["new"]), set(&["old"]))],
                relation_differs: vec![path("broken.jpg")],
                missing: vec![path("missing.jpg")],
                extra: vec![path("extra.jpg")],
            }
        );
        let report = check.report();
        assert!(report.contains("4 image(s) checked"));
        assert!(report.contains("differs.jpg — shown [new], catalog [old]"));
        assert!(!check.is_clean());
        // In agreement: clean.
        let agree = vec![(path("same.jpg"), set(&["a", "b"]))];
        let mut rows = c.tag_rows(&folder, Depth::Children).unwrap();
        rows.retain(|k, _| *k == key(&path("same.jpg")));
        let clean = Check::compare(&agree, rows);
        assert!(clean.is_clean() && clean.report().ends_with("No differences."));
    }
    #[test]
    fn rows_of_the_existing_keyword_index_import_with_matching_stamps() {
        use crate::keyword_index::{CachedKeywords, Fingerprint};
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("photos");
        fs::create_dir_all(&folder).unwrap();
        let (plain, with_sidecar) = (folder.join("a.png"), folder.join("b.png"));
        fs::write(&plain, b"pixels").unwrap();
        fs::write(&with_sidecar, b"more pixels").unwrap();
        let sidecar = crate::metadata::sidecar(&with_sidecar);
        fs::write(&sidecar, b"xmp").unwrap();
        let old = dir.path().join("keyword-index.sqlite3");
        {
            // The keyword index as NoirView before 0.1.79 wrote it.
            let c = Connection::open(&old).unwrap();
            c.execute_batch(
                "CREATE TABLE keyword_index (folder BLOB NOT NULL, recursive INTEGER NOT NULL,
                    path BLOB NOT NULL, stamp TEXT NOT NULL, tags TEXT NOT NULL,
                    PRIMARY KEY(folder,recursive,path)) WITHOUT ROWID;
                CREATE TABLE reader_revision (revision INTEGER NOT NULL);
                INSERT INTO reader_revision VALUES(3); PRAGMA user_version=1;",
            )
            .unwrap();
            // The same files under two scopes, as the viewer and the watcher store them.
            for (scope, recursive) in [(&folder, false), (&dir.path().to_path_buf(), true)] {
                for p in [&plain, &with_sidecar] {
                    let row = CachedKeywords {
                        stamp: Fingerprint::capture(p).unwrap(),
                        tags: BTreeSet::from(["moss".to_string()]),
                    };
                    c.execute(
                        "INSERT INTO keyword_index VALUES(?1,?2,?3,?4,?5)",
                        params![
                            key(scope),
                            recursive,
                            key(p),
                            serde_json::to_string(&row.stamp).unwrap(),
                            serde_json::to_string(&row.tags).unwrap()
                        ],
                    )
                    .unwrap();
                }
            }
        }
        let mut c = catalog(dir.path());
        assert_eq!(c.import_keyword_index(&old, 2).unwrap(), 0);
        assert_eq!(c.import_keyword_index(&old, 3).unwrap(), 2);
        let found = c.get(&plain).unwrap().unwrap();
        assert_eq!(
            found.image,
            Stamp::from_metadata(&fs::metadata(&plain).unwrap())
        );
        assert!(found.sidecar.is_none() && found.reusable());
        assert!(found.tags.contains("moss"));
        let found = c.get(&with_sidecar).unwrap().unwrap();
        assert_eq!(
            found.sidecar,
            Some(Stamp::from_metadata(&fs::metadata(&sidecar).unwrap()))
        );
        assert_eq!(c.ids_with_keyword("moss").unwrap().len(), 2);
    }
    #[test]
    fn checkpoints_are_rare_and_unflushed_batches_survive_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        {
            let mut c = catalog(dir.path());
            let pages: i64 = c
                .connection
                .pragma_query_value(None, "wal_autocheckpoint", |r| r.get(0))
                .unwrap();
            assert_eq!(pages, CHECKPOINT_PAGES);
            let limit: i64 = c
                .connection
                .pragma_query_value(None, "journal_size_limit", |r| r.get(0))
                .unwrap();
            assert_eq!(limit, LOG_LIMIT_BYTES);
            c.store(&[(path.clone(), entry(1, &["t"]))]).unwrap();
            // A second connection sees the committed row while it is still in the log.
            assert!(catalog(dir.path()).get(&path).unwrap().is_some());
            c.checkpoint().unwrap();
        }
        assert!(catalog(dir.path()).get(&path).unwrap().is_some());
    }
    #[test]
    fn pending_rows_flush_by_count_or_age_and_the_latest_row_for_a_path_wins() {
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let mut c = catalog(dir.path());
        let path = dir.path().join("a.jpg");
        let start = Instant::now();
        let mut pending = Pending::default();
        assert!(!pending.due(start) && pending.flush(&mut c).unwrap() == 0);
        pending.push(path.clone(), entry(1, &["old"]), start);
        pending.push(
            path.clone(),
            entry(2, &["new"]),
            start + Duration::from_secs(1),
        );
        assert!(!pending.due(start + Duration::from_secs(1)));
        // Not stored yet, so not visible.
        assert!(c.get(&path).unwrap().is_none());
        assert!(pending.due(start + Pending::AGE));
        assert_eq!(pending.flush(&mut c).unwrap(), 2);
        assert!(pending.is_empty() && !pending.due(start + Duration::from_secs(60)));
        assert_eq!(c.get(&path).unwrap(), Some(entry(2, &["new"])));
        assert!(c.ids_with_keyword("old").unwrap().is_empty());
        for index in 0..Pending::ROWS {
            pending.push(
                dir.path().join(format!("{index}.jpg")),
                entry(1, &[]),
                start,
            );
        }
        assert!(pending.due(start));
    }
    #[test]
    fn a_newer_catalog_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.sqlite3");
        drop(Catalog::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA + 1)
            .unwrap();
        assert!(Catalog::open(&path).is_err());
    }
}

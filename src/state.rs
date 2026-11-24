//! Durable user state. One bounded writer serializes changes, backups and restore.
//! This database is not the disposable keyword index.
use crate::{
    identity::{self, MarkRecord, Snapshot},
    model::{Library, PhotoMeta},
    storage::Store,
};
use anyhow::{bail, ensure, Context, Result};
use crossbeam_channel::{bounded, Sender, TrySendError};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
#[cfg(test)]
#[path = "identity_state_tests.rs"]
mod identity_state_tests;
#[cfg(test)]
mod scale_bench;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    thread::JoinHandle,
    time::Duration,
};

pub fn database_path(store: &Store) -> PathBuf {
    store.path.with_file_name("library.sqlite3")
}

fn configure(connection: &Connection) -> Result<()> {
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.execute_batch("PRAGMA foreign_keys=ON;")?;
    Ok(())
}
fn version(connection: &Connection) -> Result<i64> {
    Ok(connection.pragma_query_value(None, "user_version", |r| r.get(0))?)
}
fn open(store: &Store) -> Result<Connection> {
    open_with_seed(store, None)
}
fn open_with_seed(store: &Store, seed: Option<&Library>) -> Result<Connection> {
    let database = database_path(store);
    std::fs::create_dir_all(database.parent().context("Missing state directory")?)?;
    let mut connection = Connection::open(database)?;
    configure(&connection)?;
    let v = version(&connection)?;
    ensure!(v <= 1, "User database belongs to a newer NoirView version");
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
    if v == 0 {
        // An interrupted import is retried from the untouched legacy file.
        let legacy;
        let initial = if let Some(seed) = seed {
            seed
        } else {
            legacy = store.load_legacy()?;
            &legacy
        };
        let tx = connection.transaction()?;
        tx.execute_batch("CREATE TABLE app_state(id INTEGER PRIMARY KEY CHECK(id=1), header TEXT NOT NULL);
            CREATE TABLE images(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE,
                favorite INTEGER NOT NULL, elite INTEGER NOT NULL);
            CREATE TABLE keywords(id INTEGER PRIMARY KEY, keyword TEXT NOT NULL UNIQUE);
            CREATE TABLE image_keywords(image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
                keyword_id INTEGER NOT NULL REFERENCES keywords(id), PRIMARY KEY(image_id,keyword_id)) WITHOUT ROWID;
            CREATE INDEX image_keywords_by_keyword ON image_keywords(keyword_id,image_id);
            CREATE TABLE image_identity(image_id INTEGER PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE, identity TEXT NOT NULL);
            CREATE TABLE file_marks(identity TEXT PRIMARY KEY, record TEXT NOT NULL) WITHOUT ROWID;
            PRAGMA user_version=1;")?;
        write_header(&tx, initial)?;
        for (path, meta) in initial.photos.iter() {
            write_photo(&tx, path, Some(meta))?;
        }
        for (id, record) in initial.tracked_marks.iter() {
            write_mark(&tx, id, Some(record))?;
        }
        tx.commit()?;
    } else {
        // Missing state is corruption, never permission to replace user data.
        let _: String =
            connection.query_row("SELECT header FROM app_state WHERE id=1", [], |r| r.get(0))?;
        let tx = connection.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS image_identity(image_id INTEGER PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE, identity TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS file_marks(identity TEXT PRIMARY KEY, record TEXT NOT NULL) WITHOUT ROWID;")?;
        tx.commit()?;
    }
    Ok(connection)
}
pub fn load(store: &Store) -> Result<Library> {
    let connection =
        Connection::open_with_flags(database_path(store), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    configure(&connection)?;
    let v = version(&connection)?;
    if v == 0 {
        let tables: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            tables == 0,
            "Incomplete user database; the original has been preserved"
        );
        return store.load_legacy();
    }
    ensure!(v == 1, "Unsupported user database version");
    read_library(&connection)
}
fn read_library(connection: &Connection) -> Result<Library> {
    // Header and rows are one read snapshot, even if another instance saves.
    let tx = connection.unchecked_transaction()?;
    let header: String =
        tx.query_row("SELECT header FROM app_state WHERE id=1", [], |r| r.get(0))?;
    let mut library: Library = serde_json::from_str(&header)?;
    ensure!(library.schema <= 1, "Unsupported library version");
    {
        let identities: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='image_identity' AND type='table')",[],|r|r.get(0))?;
        let sql = if identities {
            "SELECT i.path,i.favorite,i.elite,k.keyword,fi.identity FROM images i
            LEFT JOIN image_identity fi ON fi.image_id=i.id
            LEFT JOIN image_keywords ik ON ik.image_id=i.id
            LEFT JOIN keywords k ON k.id=ik.keyword_id ORDER BY i.id"
        } else {
            "SELECT i.path,i.favorite,i.elite,k.keyword,NULL FROM images i
            LEFT JOIN image_keywords ik ON ik.image_id=i.id
            LEFT JOIN keywords k ON k.id=ik.keyword_id ORDER BY i.id"
        };
        let mut query = tx.prepare(sql)?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let encoded: String = row.get(0)?;
            let path: PathBuf = serde_json::from_str(&encoded)?;
            let meta = library.photos.entry(path).or_default();
            meta.favorite = row.get(1)?;
            meta.elite = row.get(2)?;
            meta.identity = row.get(4)?;
            if let Some(keyword) = row.get::<_, Option<String>>(3)? {
                meta.tags.insert(keyword);
            }
        }
    }
    let tracked: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='file_marks' AND type='table')",
        [],
        |r| r.get(0),
    )?;
    if tracked {
        let mut q = tx.prepare("SELECT identity,record FROM file_marks")?;
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let record: String = row.get(1)?;
            library
                .tracked_marks
                .set(id, Some(serde_json::from_str(&record)?));
        }
    }
    library.sync_marks(None);
    library.photos.clear_changes();
    library.tracked_marks.clear_changes();
    tx.commit()?;
    Ok(library)
}
fn write_header(connection: &Connection, library: &Library) -> Result<()> {
    connection.execute(
        "INSERT INTO app_state(id,header) VALUES(1,?1)
        ON CONFLICT(id) DO UPDATE SET header=excluded.header",
        [serde_json::to_string(&library.header())?],
    )?;
    Ok(())
}
fn write_photo(connection: &Connection, path: &Path, meta: Option<&PhotoMeta>) -> Result<()> {
    let real_path = path;
    // JSON path encoding retains the existing backup format's platform behavior.
    let path = serde_json::to_string(path)?;
    let previous=connection.query_row("SELECT fi.identity,i.favorite,i.elite FROM images i JOIN image_identity fi ON fi.image_id=i.id WHERE i.path=?1",[&path],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,bool>(2)?))).optional()?;
    let Some(meta) = meta else {
        if let Some((id, _, _)) = &previous {
            remove_owned_mark(connection, id, real_path)?;
        }
        connection
            .prepare_cached("DELETE FROM images WHERE path=?1")?
            .execute([path])?;
        return Ok(());
    };
    connection
        .prepare_cached(
            "INSERT INTO images(path,favorite,elite) VALUES(?1,?2,?3)
        ON CONFLICT(path) DO UPDATE SET favorite=excluded.favorite,elite=excluded.elite",
        )?
        .execute(params![path, meta.favorite, meta.elite])?;
    let id: i64 = connection
        .prepare_cached("SELECT id FROM images WHERE path=?1")?
        .query_row([&path], |r| r.get(0))?;
    let identity = if meta.rebound_from.is_some() {
        identity::capture(real_path)
    } else {
        meta.identity.clone().or_else(|| {
            (meta.favorite || meta.elite)
                .then(|| {
                    previous
                        .as_ref()
                        .map(|p| p.0.clone())
                        .or_else(|| identity::capture(real_path))
                })
                .flatten()
        })
    };
    if let Some(old) = &meta.rebound_from {
        for old in std::iter::once(old).chain(previous.as_ref().map(|p| &p.0)) {
            if identity.as_ref() != Some(old) {
                remove_owned_mark(connection, old, real_path)?;
            }
        }
    }
    if let Some(identity) = identity {
        connection.execute("INSERT INTO image_identity(image_id,identity) VALUES(?1,?2) ON CONFLICT(image_id) DO UPDATE SET identity=excluded.identity",params![id,identity])?;
        let owned: Option<String> = connection
            .query_row(
                "SELECT record FROM file_marks WHERE identity=?1",
                [&identity],
                |r| r.get(0),
            )
            .optional()?;
        let eligible = meta.rebound_from.is_some()
            || owned
                .map(|s| serde_json::from_str::<MarkRecord>(&s))
                .transpose()?
                .is_none_or(|m| m.path == real_path);
        if eligible {
            if meta.favorite || meta.elite {
                write_mark(
                    connection,
                    &identity,
                    Some(&MarkRecord {
                        path: real_path.into(),
                        favorite: meta.favorite,
                        elite: meta.elite,
                    }),
                )?;
            } else {
                write_mark(connection, &identity, None)?;
            }
        }
    } else {
        connection.execute("DELETE FROM image_identity WHERE image_id=?1", [id])?;
        if let Some((identity, favorite, elite)) = previous {
            if favorite || elite {
                remove_owned_mark(connection, &identity, real_path)?;
            }
        }
    }
    connection
        .prepare_cached("DELETE FROM image_keywords WHERE image_id=?1")?
        .execute([id])?;
    for tag in &meta.tags {
        connection
            .prepare_cached(
                "INSERT INTO keywords(keyword) VALUES(?1) ON CONFLICT(keyword) DO NOTHING",
            )?
            .execute([tag])?;
        connection
            .prepare_cached(
                "INSERT INTO image_keywords(image_id,keyword_id)
            SELECT ?1,id FROM keywords WHERE keyword=?2",
            )?
            .execute(params![id, tag])?;
    }
    Ok(())
}
fn write_mark(connection: &Connection, id: &str, record: Option<&MarkRecord>) -> Result<()> {
    if let Some(record) = record {
        connection.execute("INSERT INTO file_marks(identity,record) VALUES(?1,?2) ON CONFLICT(identity) DO UPDATE SET record=excluded.record",params![id,serde_json::to_string(record)?])?;
    } else {
        connection.execute("DELETE FROM file_marks WHERE identity=?1", [id])?;
    }
    Ok(())
}
fn remove_owned_mark(connection: &Connection, id: &str, path: &Path) -> Result<()> {
    let record: Option<String> = connection
        .query_row(
            "SELECT record FROM file_marks WHERE identity=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if record
        .map(|s| serde_json::from_str::<MarkRecord>(&s))
        .transpose()?
        .is_some_and(|r| r.path == path)
    {
        write_mark(connection, id, None)?;
    }
    Ok(())
}
pub fn replace(store: &Store, library: &Library) -> Result<()> {
    let mut connection = open_with_seed(store, Some(library))?;
    replace_connection(&mut connection, library)
}
fn replace_connection(connection: &mut Connection, library: &Library) -> Result<()> {
    let tx = connection.transaction()?;
    tx.execute_batch("DELETE FROM image_keywords; DELETE FROM images; DELETE FROM keywords; DELETE FROM file_marks;")?;
    write_header(&tx, library)?;
    for (path, meta) in library.photos.iter() {
        write_photo(&tx, path, Some(meta))?;
    }
    for (id, record) in library.tracked_marks.iter() {
        write_mark(&tx, id, Some(record))?;
    }
    tx.commit()?;
    Ok(())
}
fn preserve_unreadable(store: &Store) -> Result<()> {
    let parent = store.path.parent().context("Missing state directory")?;
    let copy = tempfile::Builder::new()
        .prefix("before-restore-unreadable-")
        .tempdir_in(parent)?;
    for path in [
        store.path.clone(),
        database_path(store),
        database_path(store).with_extension("sqlite3-wal"),
        database_path(store).with_extension("sqlite3-shm"),
    ] {
        if path.exists() {
            let target = copy
                .path()
                .join(path.file_name().context("Missing filename")?);
            std::fs::copy(path, &target)?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(target)?
                .sync_all()?;
        }
    }
    let _ = copy.keep();
    Ok(())
}
#[derive(Default)]
struct Patch {
    header: Option<Library>,
    photos: BTreeMap<PathBuf, Option<PhotoMeta>>,
    marks: BTreeMap<String, Option<MarkRecord>>,
}
impl Patch {
    fn capture(&mut self, library: &mut Library) {
        library.sync_marks(None);
        self.header = Some(library.header());
        self.merge_photos(library.photos.take_changes());
        self.marks.extend(library.tracked_marks.take_changes());
    }
    fn merge(&mut self, newer: Self) {
        if newer.header.is_some() {
            self.header = newer.header;
        }
        self.merge_photos(newer.photos);
        self.marks.extend(newer.marks);
    }
    fn merge_photos(&mut self, photos: BTreeMap<PathBuf, Option<PhotoMeta>>) {
        for (path, mut meta) in photos {
            if let Some(new) = meta.as_mut() {
                if new.rebound_from.is_none() {
                    if let Some(old) = self
                        .photos
                        .get(&path)
                        .and_then(|m| m.as_ref())
                        .and_then(|m| m.rebound_from.clone())
                    {
                        new.rebound_from = Some(old);
                        new.identity = None;
                    }
                }
            }
            self.photos.insert(path, meta);
        }
    }
    fn apply(&self, connection: &mut Connection) -> Result<()> {
        let tx = connection.transaction()?;
        if let Some(header) = &self.header {
            write_header(&tx, header)?;
        }
        let mut rebound = std::collections::BTreeSet::new();
        for (path, meta) in &self.photos {
            if let Some(old) = meta.as_ref().and_then(|m| m.rebound_from.as_ref()) {
                rebound.insert(old.clone());
                let prior:Option<String>=tx.query_row("SELECT fi.identity FROM images i JOIN image_identity fi ON fi.image_id=i.id WHERE i.path=?1",[serde_json::to_string(path)?],|r|r.get(0)).optional()?;
                if let Some(prior) = prior {
                    rebound.insert(prior);
                }
            }
            write_photo(&tx, path, meta.as_ref())?;
        }
        for (id, record) in &self.marks {
            if !rebound.contains(id) {
                write_mark(&tx, id, record.as_ref())?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
enum Action {
    Identify(Arc<Snapshot>, u64),
    Save,
    Flush,
    Backup(Option<String>),
    Restore(PathBuf, bool),
}
struct Request {
    patch: Patch,
    action: Action,
}
pub enum Notice {
    Identified(Result<identity::Plan, String>, u64),
    Saved(Result<(), String>, bool),
    BackedUp(Result<PathBuf, String>, Option<String>),
    Restored(Result<Box<Library>, String>),
}
pub struct StateWriter {
    tx: Sender<Request>,
    pending: Patch,
    thread: Option<JoinHandle<()>>,
}
impl StateWriter {
    pub fn new(store: Store, notify: impl Fn(Notice) + Send + 'static) -> Self {
        let (tx, rx) = bounded::<Request>(2);
        let thread = std::thread::spawn(move || {
            let mut lock = None;
            let mut connection = None;
            let mut pending = Patch::default();
            while let Ok(request) = rx.recv() {
                pending.merge(request.patch);
                let lock_result = (|| -> Result<()> {
                    if lock.is_none() {
                        let path = store.path.with_file_name("library.lock");
                        std::fs::create_dir_all(path.parent().context("Missing state directory")?)?;
                        let file = std::fs::OpenOptions::new()
                            .create(true)
                            .truncate(false)
                            .read(true)
                            .write(true)
                            .open(path)?;
                        fs2::FileExt::try_lock_exclusive(&file)
                            .context("Another NoirView instance is using this library")?;
                        lock = Some(file);
                    }
                    Ok(())
                })();
                if let Err(e) = lock_result {
                    let error = format!("{e:#}");
                    match request.action {
                        Action::Identify(_, token) => notify(Notice::Identified(Err(error), token)),
                        Action::Save => notify(Notice::Saved(Err(error), false)),
                        Action::Flush => notify(Notice::Saved(Err(error), true)),
                        Action::Backup(slot) => notify(Notice::BackedUp(Err(error), slot)),
                        Action::Restore(_, _) => notify(Notice::Restored(Err(error))),
                    }
                    continue;
                }
                if let Action::Restore(path, true) = &request.action {
                    connection = None;
                    let result = (|| -> Result<Box<Library>> {
                        // Explicit repair preserves raw files before touching an unreadable state.
                        preserve_unreadable(&store)?;
                        Ok(Box::new(store.restore(path, &Library::default())?))
                    })()
                    .map_err(|e| format!("{e:#}"));
                    if result.is_ok() {
                        pending = Patch::default();
                    }
                    notify(Notice::Restored(result));
                    continue;
                }
                let result = (|| -> Result<()> {
                    if connection.is_none() {
                        connection = Some(open(&store)?);
                    }
                    pending.apply(connection.as_mut().unwrap())?;
                    pending = Patch::default();
                    Ok(())
                })()
                .map_err(|e| format!("{e:#}"));
                match request.action {
                    Action::Identify(snapshot, token) => {
                        let plan = result.and_then(|()| {
                            read_library(connection.as_ref().unwrap())
                                .map(|lib| identity::plan(&lib, snapshot))
                                .map_err(|e| format!("{e:#}"))
                        });
                        notify(Notice::Identified(plan, token));
                    }
                    Action::Save => notify(Notice::Saved(result, false)),
                    Action::Flush => notify(Notice::Saved(result, true)),
                    Action::Backup(slot) => {
                        let result = result.and_then(|()| {
                            (|| -> Result<PathBuf> {
                                let mut library = read_library(connection.as_ref().unwrap())?;
                                if let Some(slot) = &slot {
                                    library.last_backup_slot = Some(slot.clone());
                                }
                                store.backup(&library)
                            })()
                            .map_err(|e| format!("{e:#}"))
                        });
                        notify(Notice::BackedUp(result, slot));
                    }
                    Action::Restore(path, _) => {
                        let result = result.and_then(|()| {
                            (|| -> Result<Box<Library>> {
                                let current = read_library(connection.as_ref().unwrap())?;
                                // Restore uses the same sole writer and transaction path as saves.
                                Ok(Box::new(store.restore(&path, &current)?))
                            })()
                            .map_err(|e| format!("{e:#}"))
                        });
                        notify(Notice::Restored(result));
                    }
                }
            }
        });
        Self {
            tx,
            pending: Patch::default(),
            thread: Some(thread),
        }
    }
    fn request(&mut self, library: &mut Library, action: Action) -> Result<()> {
        self.pending.capture(library);
        let request = Request {
            patch: std::mem::take(&mut self.pending),
            action,
        };
        match self.tx.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(request)) => {
                self.pending = request.patch;
                bail!("State writer is busy; changes remain queued for retry")
            }
            Err(TrySendError::Disconnected(request)) => {
                self.pending = request.patch;
                bail!("State writer stopped; changes could not be saved")
            }
        }
    }
    pub fn save(&mut self, library: &mut Library) -> Result<()> {
        self.request(library, Action::Save)
    }
    pub fn identify(
        &mut self,
        library: &mut Library,
        snapshot: Arc<Snapshot>,
        token: u64,
    ) -> Result<()> {
        self.request(library, Action::Identify(snapshot, token))
    }
    pub fn flush(&mut self, library: &mut Library) -> Result<()> {
        self.request(library, Action::Flush)
    }
    pub fn backup(&mut self, library: &mut Library, slot: Option<String>) -> Result<()> {
        self.request(library, Action::Backup(slot))
    }
    pub fn restore(&mut self, library: &mut Library, path: PathBuf, repair: bool) -> Result<()> {
        if repair {
            let request = Request {
                patch: Patch::default(),
                action: Action::Restore(path, true),
            };
            self.tx
                .try_send(request)
                .map_err(|_| anyhow::anyhow!("State writer is busy; try restore again"))
        } else {
            self.request(library, Action::Restore(path, false))
        }
    }
    pub fn shutdown(mut self) {
        drop(self.tx);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PhotoMeta;
    use crossbeam_channel::{unbounded, Receiver};
    use std::collections::BTreeSet;

    fn store(dir: &Path) -> Store {
        Store {
            path: dir.join("library.json"),
            exe_dir: dir.into(),
        }
    }
    fn writer(store: &Store) -> (StateWriter, Receiver<Notice>) {
        let (tx, rx) = unbounded();
        (
            StateWriter::new(store.clone(), move |notice| {
                tx.send(notice).unwrap();
            }),
            rx,
        )
    }
    fn saved(rx: &Receiver<Notice>, flush: bool) -> Result<(), String> {
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Notice::Saved(result, actual) => {
                assert_eq!(actual, flush);
                result
            }
            _ => panic!("Expected save result"),
        }
    }
    fn fixture() -> Library {
        let mut lib = Library {
            schema: 1,
            ..Default::default()
        };
        lib.photos.insert(
            "日本語 & night.png".into(),
            PhotoMeta {
                identity: None,
                rebound_from: None,
                favorite: true,
                elite: true,
                tags: BTreeSet::from(["night".into(), "日本語".into()]),
            },
        );
        lib.tags.insert("night".into());
        lib.hidden_tags.insert("hidden".into());
        lib.settings.hover = true;
        lib
    }
    #[test]
    fn manual_and_additional_palette_sources_survive_writer_flush_without_rewriting_legacy_json() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let mut lib = fixture();
        lib.create_manual_tag("own crème brûlée".into());
        lib.learn_tags(&std::collections::BTreeSet::from(["camera".into()]));
        let legacy = serde_json::to_vec(&lib).unwrap();
        std::fs::write(&store.path, &legacy).unwrap();
        let (mut writer, rx) = writer(&store);
        writer.flush(&mut lib).unwrap();
        saved(&rx, true).unwrap();
        drop(writer);
        let restored = store.load().unwrap();
        assert!(restored.is_manual_tag("own crème brûlée"));
        assert!(!restored.is_manual_tag("camera"));
        assert!(!restored.is_manual_tag("night"));
        assert_eq!(restored.tags, lib.tags);
        assert_eq!(restored.hidden_tags, lib.hidden_tags);
        assert!(restored.photos[std::path::Path::new("日本語 & night.png")].favorite);
        assert_eq!(std::fs::read(store.path).unwrap(), legacy);
    }
    #[test]
    fn display_and_prefetch_settings_survive_writer_flush_and_store_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let mut lib = fixture();
        lib.settings.prefetch_ahead = 0;
        lib.settings.wheel_up_next = false;
        lib.settings.history_limit = 24;
        lib.settings.tags_max_width = Some(1012.);
        lib.settings.prefetch_behind = 32;
        lib.settings.lens_border = false;
        lib.settings.lens_soft_edges = true;
        lib.settings.lens_feather_px = 28.;
        lib.settings.lens_full_window = true;
        lib.settings.jpeg_preview = false;
        lib.settings.display_mode = Some(crate::model::DisplayMode::Fill);
        lib.settings.thumbnail_cache_mb = 64;
        let (mut writer, rx) = writer(&store);
        writer.flush(&mut lib).unwrap();
        saved(&rx, true).unwrap();
        drop(writer);
        let restored = store.load().unwrap();
        assert_eq!(
            restored.settings.view_mode(),
            crate::model::DisplayMode::Fill
        );
        assert_eq!(restored.settings.thumbnail_cache_mb, 64);
        assert!(!restored.settings.wheel_up_next);
        assert_eq!(restored.settings.history_limit, 24);
        assert_eq!(restored.settings.tags_max_width, Some(1012.));
        assert!(!restored.settings.lens_border && !restored.settings.jpeg_preview);
        assert!(restored.settings.lens_soft_edges && restored.settings.lens_full_window);
        assert_eq!(restored.settings.lens_feather_px, 28.);
        assert_eq!(
            (
                restored.settings.prefetch_ahead,
                restored.settings.prefetch_behind
            ),
            (0, 32)
        );
        assert!(restored.photos.values().any(|p| p.favorite && p.elite));
    }
    #[test]
    fn legacy_import_preserves_original_and_normalizes_user_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let lib = fixture();
        let original = serde_json::to_vec_pretty(&lib).unwrap();
        std::fs::write(&store.path, &original).unwrap();
        let connection = open(&store).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM keywords", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            serde_json::to_value(store.load().unwrap()).unwrap(),
            serde_json::to_value(Library {
                manual_tags: Some(Default::default()),
                ..lib.clone()
            })
            .unwrap()
        );
        assert_eq!(std::fs::read(&store.path).unwrap(), original);
        assert!(store.load().unwrap().photos.take_changes().is_empty());
    }
    #[test]
    fn navigation_patch_writes_no_photo_rows_and_one_edit_preserves_ids() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let mut lib = fixture();
        for i in 0..5000 {
            lib.photos
                .insert(format!("{i}.jpg").into(), PhotoMeta::default());
        }
        store.save(&lib).unwrap();
        lib.photos.clear_changes();
        let mut connection = open(&store).unwrap();
        let id: i64 = connection
            .query_row(
                "SELECT id FROM images WHERE path=?1",
                [serde_json::to_string(Path::new("123.jpg")).unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let before = connection.total_changes();
        lib.last_image = Some("4000.jpg".into());
        let mut patch = Patch::default();
        patch.capture(&mut lib);
        assert!(patch.photos.is_empty());
        patch.apply(&mut connection).unwrap();
        assert_eq!(connection.total_changes() - before, 1);
        lib.photos.entry("123.jpg".into()).or_default().favorite = true;
        patch.capture(&mut lib);
        assert_eq!(patch.photos.len(), 1);
        patch.apply(&mut connection).unwrap();
        assert!(store.load().unwrap().photos[Path::new("123.jpg")].favorite);
        let after: i64 = connection
            .query_row(
                "SELECT id FROM images WHERE path=?1",
                [serde_json::to_string(Path::new("123.jpg")).unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id, after);
    }
    #[test]
    fn failed_transaction_rolls_back_and_retries_all_record_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save(&Library::default()).unwrap();
        let control = open(&store).unwrap();
        control.execute_batch("CREATE TRIGGER fail_save BEFORE INSERT ON images BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
        let (mut writer, rx) = writer(&store);
        let mut lib = fixture();
        writer.save(&mut lib).unwrap();
        assert!(saved(&rx, false).is_err());
        assert!(store.load().unwrap().photos.is_empty());
        assert!(!store.load().unwrap().settings.hover);
        control.execute_batch("DROP TRIGGER fail_save;").unwrap();
        // Photo dirty flags were already drained; the worker retains the failed delta.
        assert!(lib.photos.take_changes().is_empty());
        writer.flush(&mut lib).unwrap();
        saved(&rx, true).unwrap();
        assert_eq!(store.load().unwrap().photos.len(), 1);
        assert!(store.load().unwrap().settings.hover);
        writer.shutdown();
    }
    #[test]
    fn bounded_writer_coalesces_changes_without_waiting_for_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let (tx, rx) = unbounded();
        let (entered_tx, entered_rx) = bounded(1);
        let (release_tx, release_rx) = bounded(1);
        let mut first = true;
        // Block the first completion after commit to deterministically simulate a busy disk worker.
        let first_call = std::sync::Mutex::new(move || {
            if first {
                first = false;
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            }
        });
        let mut writer = StateWriter::new(store.clone(), move |notice| {
            first_call.lock().unwrap()();
            tx.send(notice).unwrap();
        });
        let mut lib = fixture();
        writer.save(&mut lib).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        lib.photos.insert("added.jpg".into(), PhotoMeta::default());
        writer.save(&mut lib).unwrap();
        lib.photos.remove(Path::new("日本語 & night.png"));
        writer.save(&mut lib).unwrap();
        for i in 0..100 {
            lib.photos.entry("added.jpg".into()).or_default().favorite = i % 2 == 1;
            lib.last_image = Some(format!("{i}.jpg").into());
            assert!(writer.save(&mut lib).is_err());
        }
        assert_eq!(writer.pending.photos.len(), 1);
        release_tx.send(()).unwrap();
        for _ in 0..3 {
            saved(&rx, false).unwrap();
        }
        writer.flush(&mut lib).unwrap();
        saved(&rx, true).unwrap();
        let actual = store.load().unwrap();
        assert_eq!(actual.last_image, lib.last_image);
        assert_eq!(*actual.photos, *lib.photos);
        writer.shutdown();
    }
    #[test]
    fn another_instance_cannot_write_and_retry_retains_unsaved_edits() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.path().join("library.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let (mut writer, rx) = writer(&store);
        let mut lib = fixture();
        writer.save(&mut lib).unwrap();
        assert!(saved(&rx, false).unwrap_err().contains("Another NoirView"));
        fs2::FileExt::unlock(&lock).unwrap();
        writer.flush(&mut lib).unwrap();
        saved(&rx, true).unwrap();
        assert_eq!(*store.load().unwrap().photos, *lib.photos);
        writer.shutdown();
    }
    #[test]
    fn backup_and_restore_are_ordered_with_saves_and_preserve_safety_copy() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let (mut writer, rx) = writer(&store);
        let mut lib = fixture();
        writer
            .backup(&mut lib, Some("2026-11-07T20:00".into()))
            .unwrap();
        let backup = match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Notice::BackedUp(Ok(path), _) => path,
            _ => panic!("Backup failed"),
        };
        lib.photos.remove(Path::new("日本語 & night.png"));
        lib.settings.hover = false;
        writer.save(&mut lib).unwrap();
        saved(&rx, false).unwrap();
        writer.restore(&mut lib, backup, false).unwrap();
        let restored = match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Notice::Restored(Ok(lib)) => lib,
            _ => panic!("Restore failed"),
        };
        assert!(restored.settings.hover);
        assert_eq!(restored.photos.len(), 1);
        assert_eq!(
            restored.last_backup_slot.as_deref(),
            Some("2026-11-07T20:00")
        );
        let safety = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("before-restore-")
            })
            .unwrap();
        let before: Library = serde_json::from_slice(&std::fs::read(safety).unwrap()).unwrap();
        assert!(before.photos.is_empty());
        assert!(!before.settings.hover);
        writer.shutdown();
    }
    #[test]
    fn explicit_restore_recovers_malformed_legacy_without_overwriting_original() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        std::fs::write(&store.path, b"broken legacy").unwrap();
        let backup = store.backup(&fixture()).unwrap();
        assert!(store.load().is_err());
        let (mut writer, rx) = writer(&store);
        let mut lib = Library::default();
        writer.restore(&mut lib, backup, true).unwrap();
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Notice::Restored(Ok(_)) => {}
            _ => panic!("Repair failed"),
        }
        assert_eq!(store.load().unwrap().photos.len(), 1);
        assert_eq!(std::fs::read(&store.path).unwrap(), b"broken legacy");
        writer.shutdown();
    }
}

//! Deliver filenames first; populate a persistent keyword index independently.
use crate::{
    keyword_index::{self, CachedKeywords, Fingerprint, KeywordIndex},
    metadata,
    model::{self, Library},
};
use anyhow::Result;
#[cfg(test)]
use crossbeam_channel::unbounded;
use crossbeam_channel::{bounded, Receiver};
#[path = "folder_changes.rs"]
mod changes;
use changes::ChangeInbox;
use notify::{
    event::{MetadataKind, ModifyKind},
    EventKind, RecursiveMode, Watcher,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const BATCH: usize = 64;
#[derive(Clone, Debug)]
pub struct Keywords {
    pub path: PathBuf,
    pub tags: BTreeSet<String>,
    pub revision: u64,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub cached: usize,
    pub read: usize,
    pub failed: usize,
    pub elapsed: Duration,
}
#[derive(Debug)]
pub struct FileSnapshot {
    pub paths: Vec<PathBuf>,
    pub members: HashSet<PathBuf>,
}
impl FileSnapshot {
    fn new(paths: &[PathBuf]) -> Self {
        Self {
            paths: paths.to_vec(),
            members: paths.iter().cloned().collect(),
        }
    }
}
#[derive(Debug)]
pub enum Update {
    Identities(Arc<crate::identity::Snapshot>),
    Files(FileSnapshot),
    ImagesChanged(Vec<PathBuf>),
    ImagesChangedAll,
    Keywords(Vec<Keywords>),
    Progress(Progress),
    Complete(Progress),
    Warning(String),
    Error(String),
}
#[derive(Default)]
struct RevisionState {
    paths: HashMap<PathBuf, u64>,
    generation: u64,
    serial: u64,
}
#[derive(Clone, Default)]
struct Revisions(Arc<Mutex<RevisionState>>);
impl Revisions {
    fn snapshot(&self, path: &Path) -> (u64, u64) {
        let state = self.0.lock().unwrap();
        (*state.paths.get(path).unwrap_or(&0), state.generation)
    }
    fn bump(&self, path: &Path) {
        let mut state = self.0.lock().unwrap();
        state.serial += 1;
        *state.paths.entry(path.to_path_buf()).or_default() += 1;
    }
    fn external(&self, path: &Path) {
        let mut state = self.0.lock().unwrap();
        state.serial += 1;
        *state.paths.entry(path.to_path_buf()).or_default() += 2;
    }
    fn reconcile(&self) {
        let mut state = self.0.lock().unwrap();
        state.generation += 1;
        state.serial += 1;
        state.paths.retain(|_, v| !v.is_multiple_of(2));
    }
    fn accepts(&self, item: &Keywords) -> bool {
        let now = self.snapshot(&item.path);
        now.0.is_multiple_of(2) && now == (item.revision, item.generation)
    }
}
pub struct FolderLoader {
    rx: Receiver<Update>,
    changes: Option<Arc<ChangeInbox>>,
    epoch: Arc<AtomicU64>,
    revisions: Revisions,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl FolderLoader {
    pub fn identity_serial(&self) -> u64 {
        self.revisions.0.lock().unwrap().serial
    }
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (_, rx) = bounded(1);
        Self {
            rx,
            changes: None,
            epoch: Arc::new(AtomicU64::new(0)),
            revisions: Revisions::default(),
            wake: Arc::new(wake),
        }
    }
    pub fn start(
        &mut self,
        folder: PathBuf,
        recursive: bool,
        database: Option<PathBuf>,
        force: bool,
    ) {
        self.start_reader(folder, recursive, database, force, metadata::read);
    }
    fn start_reader(
        &mut self,
        folder: PathBuf,
        recursive: bool,
        database: Option<PathBuf>,
        force: bool,
        read: impl FnMut(&Path) -> Result<BTreeSet<String>> + Send + 'static,
    ) {
        self.cancel();
        let token = self.epoch.load(Ordering::Relaxed);
        let epoch = self.epoch.clone();
        let (tx, rx) = bounded(8);
        self.rx = rx;
        self.revisions = Revisions::default();
        let revisions = self.revisions.clone();
        let wake = self.wake.clone();
        let changes = Arc::new(ChangeInbox::new(revisions.clone(), wake.clone()));
        self.changes = Some(changes.clone());
        std::thread::spawn(move || {
            let current = || epoch.load(Ordering::Relaxed) == token;
            let send = |update| {
                if !current() {
                    return false;
                }
                let ok = tx.send(update).is_ok();
                wake();
                ok
            };
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(
                    folder, recursive, database, force, &send, &current, &revisions, changes, read,
                )
            }));
            if outcome.is_err() {
                send(Update::Error("Keyword indexing stopped unexpectedly. The images can still be browsed; rebuild the index to retry.".into()));
            }
        });
    }
    pub fn cancel(&mut self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        let (_, rx) = bounded(1);
        self.rx = rx;
        if let Some(changes) = self.changes.take() {
            changes.stop();
        }
    }
    pub fn try_recv(&self) -> Result<Update, crossbeam_channel::TryRecvError> {
        self.changes
            .as_ref()
            .and_then(|c| c.take_pixels())
            .map(Ok)
            .unwrap_or_else(|| self.rx.try_recv())
    }
    pub fn next_wake(&self) -> Option<Duration> {
        if !self.rx.is_empty() {
            Some(Duration::ZERO)
        } else {
            self.changes.as_ref().and_then(|c| c.pixel_delay())
        }
    }
    pub fn begin_write(&self, path: &Path) {
        self.revisions.bump(path);
    }
    pub fn finish_write(&self, path: &Path) {
        self.revisions.bump(path);
        if let Some(changes) = &self.changes {
            changes.internal(path.to_path_buf());
        }
    }
    pub fn accepts(&self, item: &Keywords) -> bool {
        self.revisions.accepts(item)
    }
}
impl Drop for FolderLoader {
    fn drop(&mut self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        if let Some(changes) = &self.changes {
            changes.stop();
        }
    }
}

fn relevant(event: &notify::Event) -> bool {
    if event.need_rescan() {
        return true;
    }
    if event.kind.is_access()
        || matches!(
            event.kind,
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
        )
    {
        return false;
    }
    if matches!(
        event.kind,
        EventKind::Create(notify::event::CreateKind::Folder)
            | EventKind::Remove(notify::event::RemoveKind::Folder)
            | EventKind::Modify(ModifyKind::Name(_))
    ) {
        return true;
    }
    // Directory and rename events can change the file list. Ignore our SQLite,
    // backup and temporary-file writes when only unrelated files are involved.
    event.paths.is_empty()
        || event.paths.iter().any(|p| {
            model::supported(p)
                || p.extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case("xmp"))
                || p.extension().is_none()
        })
}
#[allow(clippy::too_many_arguments)]
fn run(
    folder: PathBuf,
    recursive: bool,
    database: Option<PathBuf>,
    force: bool,
    send: &impl Fn(Update) -> bool,
    current: &impl Fn() -> bool,
    revisions: &Revisions,
    changes: Arc<ChangeInbox>,
    mut read: impl FnMut(&Path) -> Result<BTreeSet<String>>,
) {
    let notifications = changes.clone();
    let watcher = notify::recommended_watcher(move |event| notifications.record(event));
    let _watcher = match watcher {
        Ok(mut w) => match w.watch(
            &folder,
            if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            },
        ) {
            Ok(()) => Some(w),
            Err(e) => {
                send(Update::Warning(format!(
                    "Live keyword updates unavailable: {e}"
                )));
                None
            }
        },
        Err(e) => {
            send(Update::Warning(format!(
                "Live keyword updates unavailable: {e}"
            )));
            None
        }
    };
    let mut files = match model::scan(&folder, recursive) {
        Ok(f) => f,
        Err(e) => {
            send(Update::Error(format!("{e:#}")));
            return;
        }
    };
    if !current() || !send(Update::Files(FileSnapshot::new(&files))) {
        return;
    }
    let mut index = database.and_then(|p| match KeywordIndex::open(&p) {
        Ok(i) => Some(i),
        Err(e) => {
            send(Update::Warning(format!("Keyword cache unavailable: {e:#}")));
            None
        }
    });
    let mut cached = HashMap::new();
    if let Some(db) = &mut index {
        let result = (|| -> Result<_> {
            if force {
                db.clear(&folder, recursive)?;
            }
            db.prune(
                &folder,
                recursive,
                &files.iter().map(|p| keyword_index::key(p)).collect(),
            )?;
            db.load_any(&folder, recursive)
        })();
        match result {
            Ok(rows) => cached = rows,
            Err(e) => {
                send(Update::Warning(format!(
                    "Keyword cache could not be read: {e:#}"
                )));
                index = None;
            }
        }
    }
    // A warm cache is shown immediately, before filesystem validation begins.
    for batch in files.chunks(BATCH) {
        let entries = batch
            .iter()
            .filter_map(|p| {
                let (revision, generation) = revisions.snapshot(p);
                if revision != 0 || generation != 0 {
                    return None;
                }
                cached.get(&keyword_index::key(p)).map(|row| Keywords {
                    path: p.clone(),
                    tags: row.tags.clone(),
                    revision,
                    generation,
                })
            })
            .collect::<Vec<_>>();
        if !entries.is_empty() && !send(Update::Keywords(entries)) {
            return;
        }
    }
    if !send_identities(&files, revisions, current, send) {
        return;
    }
    let mut work = IndexWork::default();
    for path in &files {
        work.schedule(path.clone(), force, false);
    }
    let mut known: HashSet<_> = files.iter().cloned().collect();
    let mut progress = Progress {
        total: work.pending.len(),
        ..Default::default()
    };
    let mut started = Instant::now();
    let mut last = started;
    let mut entries = Vec::new();
    let mut writes = Vec::new();
    let mut indexing = true;
    if !send(Update::Progress(progress)) {
        return;
    }
    while current() {
        if changes.delay().is_some_and(|d| d.is_zero()) {
            // Notifications are collected even inside a blocked reader. Reconcile
            // here between reads; new/dirty paths take priority over the initial pass.
            let incoming = changes.take();
            if let Some(warning) = incoming.warning {
                if !send(Update::Warning(warning)) {
                    return;
                }
            }
            if !indexing {
                progress = Progress::default();
                started = Instant::now();
                last = started;
                indexing = true;
            }
            let before = work.pending.len();
            let identity_dirty =
                incoming.rescan || incoming.refresh_all || !incoming.dirty.is_empty();
            if incoming.rescan {
                match model::scan(&folder, recursive) {
                    Ok(next) => {
                        if next != files {
                            let valid: HashSet<_> =
                                next.iter().map(|p| keyword_index::key(p)).collect();
                            cached.retain(|p, _| valid.contains(p));
                            for path in &next {
                                if !known.contains(path) {
                                    work.schedule(path.clone(), true, true);
                                }
                            }
                            known = next.iter().cloned().collect();
                            files = next;
                            if !send(Update::Files(FileSnapshot::new(&files))) {
                                return;
                            }
                            if let Some(db) = &mut index {
                                if let Err(e) = db.prune(&folder, recursive, &valid) {
                                    send(Update::Warning(format!("Keyword index cleanup: {e:#}")));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        send(Update::Warning(format!("Folder refresh: {e:#}")));
                    }
                }
            }
            if incoming.refresh_all {
                for path in &files {
                    work.schedule(path.clone(), true, true);
                }
            } else {
                for path in incoming.dirty {
                    if known.contains(&path) {
                        work.schedule(path, true, true);
                    }
                }
            }
            progress.total += work.pending.len().saturating_sub(before);
            if identity_dirty && !send_identities(&files, revisions, current, send) {
                return;
            }
            if !send(Update::Progress(progress)) {
                return;
            }
        }
        if let Some((path, force)) = work.pop() {
            if known.contains(&path) {
                if let Some(item) = index_one(
                    &path,
                    force,
                    &mut cached,
                    &mut writes,
                    current,
                    revisions,
                    &mut read,
                    &mut progress,
                ) {
                    entries.push(item);
                }
            }
            progress.done += 1;
            progress.elapsed = started.elapsed();
            if progress.done.is_multiple_of(BATCH) || last.elapsed() >= Duration::from_millis(100) {
                flush(&mut index, &folder, recursive, &mut writes, send);
                if !entries.is_empty() && !send(Update::Keywords(std::mem::take(&mut entries))) {
                    return;
                }
                if !send(Update::Progress(progress)) {
                    return;
                }
                last = Instant::now();
            }
            continue;
        }
        if !current() {
            return;
        }
        if let Some(delay) = changes.delay() {
            let _ = changes.signal.recv_timeout(delay);
            continue;
        }
        if indexing {
            flush(&mut index, &folder, recursive, &mut writes, send);
            if !entries.is_empty() && !send(Update::Keywords(std::mem::take(&mut entries))) {
                return;
            }
            progress.elapsed = started.elapsed();
            if !send(Update::Complete(progress)) || !current() {
                return;
            }
            indexing = false;
        }
        if changes.signal.recv().is_err() {
            return;
        }
    }
}
fn send_identities(
    files: &[PathBuf],
    revisions: &Revisions,
    current: &impl Fn() -> bool,
    send: &impl Fn(Update) -> bool,
) -> bool {
    let serial = revisions.0.lock().unwrap().serial;
    let mut snapshot = crate::identity::Snapshot {
        serial,
        ..Default::default()
    };
    for path in files {
        if !current() {
            return false;
        }
        snapshot.paths.insert(
            path.clone(),
            crate::identity::Observation {
                identity: crate::identity::capture(path),
            },
        );
    }
    if serial != revisions.0.lock().unwrap().serial {
        return true;
    }
    send(Update::Identities(Arc::new(snapshot)))
}
#[derive(Default)]
struct IndexWork {
    pending: HashMap<PathBuf, bool>,
    urgent: HashSet<PathBuf>,
    high: VecDeque<PathBuf>,
    low: VecDeque<PathBuf>,
}
impl IndexWork {
    fn schedule(&mut self, path: PathBuf, force: bool, urgent: bool) {
        let fresh = !self.pending.contains_key(&path);
        self.pending
            .entry(path.clone())
            .and_modify(|f| *f |= force)
            .or_insert(force);
        if urgent {
            if self.urgent.insert(path.clone()) {
                self.high.push_back(path);
            }
        } else if fresh {
            self.low.push_back(path);
        }
    }
    fn pop(&mut self) -> Option<(PathBuf, bool)> {
        while let Some(path) = self.high.pop_front() {
            self.urgent.remove(&path);
            if let Some(force) = self.pending.remove(&path) {
                return Some((path, force));
            }
        }
        while let Some(path) = self.low.pop_front() {
            if let Some(force) = self.pending.remove(&path) {
                return Some((path, force));
            }
        }
        None
    }
}
#[allow(clippy::too_many_arguments)]
fn index_one(
    path: &Path,
    force: bool,
    cached: &mut HashMap<Vec<u8>, CachedKeywords>,
    writes: &mut Vec<(Vec<u8>, CachedKeywords)>,
    current: &impl Fn() -> bool,
    revisions: &Revisions,
    read: &mut impl FnMut(&Path) -> Result<BTreeSet<String>>,
    progress: &mut Progress,
) -> Option<Keywords> {
    let (revision, generation) = revisions.snapshot(path);
    let key = keyword_index::key(path);
    let result = (|| -> Result<Option<BTreeSet<String>>> {
        if !revision.is_multiple_of(2) {
            return Ok(None);
        }
        let before = Fingerprint::capture(path)?;
        if !force && before.reusable() && cached.get(&key).is_some_and(|row| row.stamp == before) {
            progress.cached += 1;
            return Ok(None);
        }
        progress.read += 1;
        // Keyword reads step aside briefly while visible grid tiles wait.
        crate::disk::yield_to_grid(Duration::from_millis(200));
        let tags = read(path)?;
        if !current()
            || revisions.snapshot(path) != (revision, generation)
            || Fingerprint::capture(path)? != before
        {
            return Ok(None);
        }
        let row = CachedKeywords {
            stamp: before,
            tags: tags.clone(),
        };
        cached.insert(key.clone(), row.clone());
        writes.push((key, row));
        Ok(Some(tags))
    })();
    match result {
        Ok(Some(tags)) => Some(Keywords {
            path: path.to_path_buf(),
            tags,
            revision,
            generation,
        }),
        Err(_) => {
            progress.failed += 1;
            None
        }
        _ => None,
    }
}
fn flush(
    index: &mut Option<KeywordIndex>,
    folder: &Path,
    recursive: bool,
    writes: &mut Vec<(Vec<u8>, CachedKeywords)>,
    send: &impl Fn(Update) -> bool,
) {
    if writes.is_empty() {
        return;
    }
    if let Some(db) = index {
        if let Err(e) = db.store(folder, recursive, writes) {
            send(Update::Warning(format!(
                "Keyword index could not be saved: {e:#}"
            )));
            *index = None;
        }
    }
    writes.clear();
}
/// Apply only results from the active scope and current image revision.
pub fn apply_keywords(
    lib: &mut Library,
    paths: &HashSet<PathBuf>,
    loader: &FolderLoader,
    entries: Vec<Keywords>,
) -> bool {
    let mut changed = false;
    for item in entries {
        if !paths.contains(&item.path) || !loader.accepts(&item) {
            continue;
        }
        changed |= lib.learn_tags(&item.tags);
        let differs = lib
            .photos
            .get(&item.path)
            .map(|m| m.tags != item.tags)
            .unwrap_or(!item.tags.is_empty());
        if differs {
            lib.photos.entry(item.path).or_default().tags = item.tags;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicBool, AtomicUsize},
    };
    fn next_update(loader: &FolderLoader, timeout: Duration) -> Update {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(update) = loader.try_recv() {
                return update;
            }
            assert!(Instant::now() < deadline, "folder update timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn run_once(
        folder: &Path,
        database: &Path,
        read: impl FnMut(&Path) -> Result<BTreeSet<String>>,
    ) -> (Vec<Update>, Progress) {
        let active = AtomicBool::new(true);
        let updates = Mutex::new(vec![]);
        let revisions = Revisions::default();
        let changes = Arc::new(ChangeInbox::new(revisions.clone(), Arc::new(|| {})));
        run(
            folder.to_path_buf(),
            false,
            Some(database.to_path_buf()),
            false,
            &|u| {
                let done = matches!(u, Update::Complete(_) | Update::Error(_));
                updates.lock().unwrap().push(u);
                if done {
                    active.store(false, Ordering::Relaxed);
                }
                true
            },
            &|| active.load(Ordering::Relaxed),
            &revisions,
            changes,
            read,
        );
        let updates = updates.into_inner().unwrap();
        let complete = updates
            .iter()
            .find_map(|u| {
                if let Update::Complete(p) = u {
                    Some(*p)
                } else {
                    None
                }
            })
            .expect("index should complete");
        (updates, complete)
    }
    #[test]
    fn keywords_indexed_by_the_folder_watcher_with_subfolders_are_reused() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("photos");
        fs::create_dir_all(folder.join("sub")).unwrap();
        let paths: Vec<_> = ["a.bmp", "b.bmp", "sub/c.bmp"]
            .iter()
            .map(|n| {
                let p = folder.join(n);
                image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
                    .save(&p)
                    .unwrap();
                p
            })
            .collect();
        let database = dir.path().join("index.sqlite3");
        // The watcher stored rows under (photos, subfolders on).
        let rows: Vec<_> = paths
            .iter()
            .map(|p| {
                (
                    keyword_index::key(p),
                    CachedKeywords {
                        stamp: keyword_index::Fingerprint::capture(p).unwrap(),
                        tags: BTreeSet::from(["moss".to_string()]),
                    },
                )
            })
            .collect();
        KeywordIndex::open(&database)
            .unwrap()
            .store(&folder, true, &rows)
            .unwrap();
        // The viewer opens the folder without subfolders: nothing is read again.
        let (updates, progress) = run_once(&folder, &database, |p| {
            panic!("read again: {}", p.display())
        });
        assert_eq!((progress.read, progress.cached), (0, 2));
        let tagged: usize = updates
            .iter()
            .filter_map(|u| match u {
                Update::Keywords(rows) => {
                    Some(rows.iter().filter(|r| r.tags.contains("moss")).count())
                }
                _ => None,
            })
            .sum();
        assert_eq!(tagged, 2);
    }
    #[test]
    fn live_images_wake_while_reader_is_blocked_and_dirty_keywords_take_priority() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = ["00.bmp", "01.bmp", "99.bmp"]
            .into_iter()
            .map(|n| {
                let p = dir.path().join(n);
                image::RgbImage::from_pixel(8, 12, image::Rgb([30, 40, 50]))
                    .save(&p)
                    .unwrap();
                p
            })
            .collect();
        metadata::write(&paths[1], &BTreeSet::from(["old".into()])).unwrap();
        let (entered, ack) = bounded(1);
        let (release, wait) = bounded(1);
        let (reads, read_rx) = unbounded();
        let first = paths[0].clone();
        let mut gated = false;
        let mut loader = FolderLoader::new(|| {});
        loader.start_reader(dir.path().into(), false, None, false, move |path| {
            reads.send(path.to_path_buf()).unwrap();
            if path == first && !gated {
                gated = true;
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(8))?;
            }
            metadata::read(path)
        });
        ack.recv_timeout(Duration::from_secs(8)).unwrap();
        image::RgbImage::from_pixel(12, 8, image::Rgb([130, 140, 150]))
            .save(&paths[1])
            .unwrap();
        metadata::write(&paths[1], &BTreeSet::from(["new".into()])).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(Instant::now() < deadline);
            if let Update::ImagesChanged(changed) = next_update(&loader, Duration::from_secs(8)) {
                if changed.contains(&paths[1]) {
                    break;
                }
            }
        }
        // No metadata read has completed. Pixel invalidation comes independently.
        assert_eq!(
            read_rx.try_iter().collect::<Vec<_>>(),
            vec![paths[0].clone()]
        );
        release.send(()).unwrap();
        let mut tags = None;
        loop {
            match next_update(&loader, Duration::from_secs(8)) {
                Update::Keywords(rows) => {
                    for row in rows {
                        if row.path == paths[1] && loader.accepts(&row) {
                            tags = Some(row.tags);
                        }
                    }
                }
                Update::Complete(_) => break,
                _ => {}
            }
        }
        assert_eq!(tags, Some(BTreeSet::from(["new".into()])));
        let reads: Vec<_> = read_rx.try_iter().collect();
        assert_eq!(reads.first(), Some(&paths[1]));
        assert!(
            reads.iter().position(|p| p == &paths[1]).unwrap()
                < reads.iter().position(|p| p == &paths[2]).unwrap()
        );
    }
    #[test]
    fn queued_warm_keywords_cannot_overwrite_a_timestamp_preserving_sidecar_edit() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("00.bmp");
        let changed = dir.path().join("01.bmp");
        for path in [&first, &changed] {
            image::RgbImage::from_pixel(8, 12, image::Rgb([30, 40, 50]))
                .save(path)
                .unwrap();
        }
        metadata::write(&changed, &BTreeSet::from(["old".into()])).unwrap();
        let database = dir.path().join("index.sqlite3");
        run_once(dir.path(), &database, metadata::read);
        image::RgbImage::from_pixel(12, 8, image::Rgb([80, 40, 50]))
            .save(&first)
            .unwrap();
        let (entered, ack) = bounded(1);
        let (release, wait) = bounded(1);
        let mut gated = false;
        let mut loader = FolderLoader::new(|| {});
        // Force the slow verification pass while the existing cache is queued.
        loader.start_reader(
            dir.path().into(),
            false,
            Some(database),
            false,
            move |path| {
                if !gated {
                    gated = true;
                    entered.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(8))?;
                }
                metadata::read(path)
            },
        );
        ack.recv_timeout(Duration::from_secs(8)).unwrap();
        let side = metadata::sidecar(&changed);
        let stamp = fs::metadata(&side).unwrap().modified().unwrap();
        let bytes = fs::read_to_string(&side).unwrap().replace(">old<", ">new<");
        fs::write(&side, bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&side)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while loader.revisions.snapshot(&changed).0 == 0 {
            assert!(
                Instant::now() < deadline,
                "watch event not collected during blocked read"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let stale: Vec<_> = loader
            .rx
            .try_iter()
            .filter_map(|u| {
                if let Update::Keywords(rows) = u {
                    Some(rows)
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        assert!(stale
            .iter()
            .any(|r| r.path == changed && r.tags.contains("old")));
        assert!(stale
            .iter()
            .filter(|r| r.path == changed)
            .all(|r| !loader.accepts(r)));
        release.send(()).unwrap();
        let mut fresh = None;
        loop {
            match next_update(&loader, Duration::from_secs(8)) {
                Update::Keywords(rows) => {
                    for row in rows {
                        if row.path == changed && loader.accepts(&row) {
                            fresh = Some(row.tags);
                        }
                    }
                }
                Update::Complete(_) => break,
                _ => {}
            }
        }
        assert_eq!(fresh, Some(BTreeSet::from(["new".into()])));
    }
    #[test]
    fn overflow_reconciles_files_and_keywords_after_a_blocked_initial_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for n in 0..4200 {
            let path = dir.path().join(format!("{n:05}.png"));
            fs::write(&path, b"old").unwrap();
            paths.push(path);
        }
        let (entered, ack) = bounded(1);
        let (release, wait) = bounded(1);
        let mut gated = false;
        let mut loader = FolderLoader::new(|| {});
        loader.start_reader(dir.path().into(), false, None, false, move |path| {
            let value = fs::read_to_string(path)?;
            if !gated {
                gated = true;
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(8))?;
            }
            Ok(BTreeSet::from([value]))
        });
        ack.recv_timeout(Duration::from_secs(8)).unwrap();
        let stamp = fs::metadata(&paths[0]).unwrap().modified().unwrap();
        fs::write(&paths[0], b"new").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&paths[0])
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        let removed = paths.pop().unwrap();
        fs::remove_file(&removed).unwrap();
        let added = dir.path().join("added.png");
        fs::write(&added, b"added").unwrap();
        let inbox = loader.changes.as_ref().unwrap();
        // A second large burst while reconciliation is pending stays bounded.
        for path in &paths {
            inbox.record(Ok(notify::Event::new(EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Any,
            )))
            .add_path(path.clone())));
        }
        let old = Keywords {
            path: paths[0].clone(),
            tags: BTreeSet::from(["old".into()]),
            revision: 0,
            generation: 0,
        };
        assert!(!loader.accepts(&old));
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(Instant::now() < deadline);
            if matches!(
                next_update(&loader, Duration::from_secs(8)),
                Update::ImagesChangedAll
            ) {
                break;
            }
        }
        release.send(()).unwrap();
        let mut files = Vec::new();
        let mut tags = HashMap::new();
        loop {
            match next_update(&loader, Duration::from_secs(12)) {
                Update::Files(rows) => files = rows.paths,
                Update::Keywords(rows) => {
                    for row in rows {
                        if loader.accepts(&row) {
                            tags.insert(row.path, row.tags);
                        }
                    }
                }
                Update::Complete(_) => break,
                _ => {}
            }
        }
        let expected = model::scan(dir.path(), false).unwrap();
        assert_eq!(files, expected);
        assert_eq!(tags.len(), expected.len());
        assert_eq!(tags[&paths[0]], BTreeSet::from(["new".into()]));
        assert_eq!(tags[&added], BTreeSet::from(["added".into()]));
        assert!(!tags.contains_key(&removed));
    }
    #[test]
    fn watcher_keeps_collecting_when_ui_result_channel_is_full() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = HashSet::new();
        for n in 0..600 {
            let path = dir.path().join(format!("{n:04}.png"));
            fs::write(&path, b"old").unwrap();
            paths.insert(path);
        }
        let changed = dir.path().join("0000.png");
        let mut loader = FolderLoader::new(|| {});
        loader.start_reader(dir.path().into(), false, None, false, move |path| {
            Ok(BTreeSet::from([fs::read_to_string(path)?]))
        });
        let deadline = Instant::now() + Duration::from_secs(8);
        while loader.rx.len() < 8 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        fs::write(&changed, b"new").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if loader
                .changes
                .as_ref()
                .unwrap()
                .pixel_delay()
                .is_some_and(|d| d.is_zero())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "watch callback blocked behind indexing/UI channel"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(loader.rx.len(), 8);
        assert!(
            matches!(loader.try_recv().unwrap(),Update::ImagesChanged(p) if p.contains(&changed))
        );
        let mut library = Library::default();
        library.photos.entry(changed.clone()).or_default().favorite = true;
        loop {
            match next_update(&loader, Duration::from_secs(8)) {
                Update::Keywords(rows) => {
                    apply_keywords(&mut library, &paths, &loader, rows);
                }
                Update::Complete(_) => break,
                _ => {}
            }
        }
        assert_eq!(
            library.photos[&changed].tags,
            BTreeSet::from(["new".into()])
        );
        assert!(library.photos[&changed].favorite);
    }
    #[test]
    fn cancelling_an_idle_loader_wakes_its_event_wait() {
        struct Finished(crossbeam_channel::Sender<()>);
        impl Drop for Finished {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.png"), b"a").unwrap();
        let (done, exited) = bounded(1);
        let guard = Finished(done);
        let mut loader = FolderLoader::new(|| {});
        loader.start_reader(dir.path().into(), false, None, false, move |_| {
            let _ = &guard;
            Ok(BTreeSet::new())
        });
        while !matches!(
            next_update(&loader, Duration::from_secs(8)),
            Update::Complete(_)
        ) {}
        loader.cancel();
        exited.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    #[test]
    fn five_thousand_paths_arrive_before_a_blocked_keyword_reader() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..5000 {
            fs::write(dir.path().join(format!("{n:08}.png")), []).unwrap();
        }
        let (tx, rx) = unbounded();
        let (entered_tx, entered_rx) = bounded(1);
        let (permit_tx, permit_rx) = bounded(1);
        let active = Arc::new(AtomicBool::new(true));
        let working = active.clone();
        let path = dir.path().to_path_buf();
        let worker = std::thread::spawn(move || {
            let revisions = Revisions::default();
            let changes = Arc::new(ChangeInbox::new(revisions.clone(), Arc::new(|| {})));
            run(
                path,
                false,
                None,
                false,
                &|u| tx.send(u).is_ok(),
                &|| working.load(Ordering::Relaxed),
                &revisions,
                changes,
                |_| {
                    entered_tx.send(()).ok();
                    permit_rx.recv_timeout(Duration::from_secs(5))?;
                    Ok(BTreeSet::new())
                },
            );
        });
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(first,Update::Files(ref paths) if paths.paths.len()==5000));
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!rx.try_iter().any(|u| matches!(u, Update::Keywords(_))));
        active.store(false, Ordering::Relaxed);
        permit_tx.send(()).unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn persistent_index_reuses_all_5000_images_and_only_reads_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        let images = dir.path().join("images");
        fs::create_dir(&images).unwrap();
        for n in 0..5000 {
            fs::write(images.join(format!("{n:08}.png")), []).unwrap();
        }
        let db = dir.path().join("cache.sqlite3");
        let reads = AtomicUsize::new(0);
        let read = |_: &Path| {
            reads.fetch_add(1, Ordering::Relaxed);
            Ok(BTreeSet::from(["indexed".into()]))
        };
        let (_, cold) = run_once(&images, &db, read);
        assert_eq!(cold.read, 5000);
        assert_eq!(cold.failed, 0);
        let before = reads.load(Ordering::Relaxed);
        let (updates, warm) = run_once(&images, &db, read);
        assert_eq!(warm.cached, 5000);
        assert_eq!(warm.read, 0);
        assert_eq!(reads.load(Ordering::Relaxed), before);
        assert!(updates.iter().any(
            |u| matches!(u,Update::Keywords(rows) if rows.iter().any(|r|r.tags.contains("indexed")))
        ));
        fs::write(images.join("00000003.png"), b"changed").unwrap();
        let (_, changed) = run_once(&images, &db, read);
        assert_eq!(changed.read, 1);
        assert_eq!(changed.cached, 4999);
        fs::write(
            metadata::sidecar(&images.join("00000004.png")),
            b"changed sidecar",
        )
        .unwrap();
        let (_, sidecar) = run_once(&images, &db, read);
        assert_eq!(sidecar.read, 1);
        fs::remove_file(images.join("00000005.png")).unwrap();
        let (_, removed) = run_once(&images, &db, read);
        assert_eq!(removed.total, 4999);
        assert_eq!(removed.read, 0);
        eprintln!("5000-image index: cold {:.3}s ({} metadata reads); warm {:.3}s ({} metadata reads); changed: {} read / {} cached",cold.elapsed.as_secs_f64(),cold.read,warm.elapsed.as_secs_f64(),warm.read,changed.read,changed.cached);
    }
    #[test]
    fn stale_index_results_cannot_overwrite_edits_or_readd_removed_images() {
        let loader = FolderLoader::new(|| {});
        let path = PathBuf::from("image.png");
        let paths = HashSet::from([path.clone()]);
        let mut lib = Library::default();
        let old = Keywords {
            path: path.clone(),
            tags: BTreeSet::from(["old".into()]),
            revision: 0,
            generation: 0,
        };
        loader.begin_write(&path);
        assert!(!apply_keywords(
            &mut lib,
            &paths,
            &loader,
            vec![old.clone()]
        ));
        let during = Keywords {
            revision: 1,
            ..old.clone()
        };
        loader.finish_write(&path);
        lib.photos.entry(path.clone()).or_default().tags = BTreeSet::from(["edited".into()]);
        assert!(!apply_keywords(
            &mut lib,
            &paths,
            &loader,
            vec![old, during]
        ));
        assert!(lib.photos[&path].tags.contains("edited"));
        let fresh = Keywords {
            path: path.clone(),
            tags: BTreeSet::from(["external edit".into()]),
            revision: 2,
            generation: 0,
        };
        assert!(apply_keywords(
            &mut lib,
            &paths,
            &loader,
            vec![fresh.clone()]
        ));
        lib.photos.remove(&path);
        assert!(!apply_keywords(
            &mut lib,
            &HashSet::new(),
            &loader,
            vec![fresh]
        ));
        assert!(!lib.photos.contains_key(&path));
    }
    #[test]
    fn switching_folders_discards_the_old_index_stream() {
        let first = tempfile::tempdir().unwrap();
        for n in 0..1000 {
            fs::write(first.path().join(format!("{n}.bmp")), []).unwrap();
        }
        let second = tempfile::tempdir().unwrap();
        fs::write(second.path().join("new.bmp"), []).unwrap();
        let mut loader = FolderLoader::new(|| {});
        loader.start(first.path().into(), false, None, false);
        while !matches!(
            loader.rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Update::Files(_)
        ) {}
        loader.start(second.path().into(), false, None, false);
        loop {
            match loader.rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Update::Files(paths) => {
                    assert_eq!(paths.paths, vec![second.path().join("new.bmp")])
                }
                Update::Keywords(rows) => {
                    assert!(rows.iter().all(|r| r.path.starts_with(second.path())))
                }
                Update::Complete(p) => {
                    assert_eq!(p.total, 1);
                    break;
                }
                Update::Error(e) => panic!("{e}"),
                _ => {}
            }
        }
    }
    #[test]
    fn live_watcher_delivers_image_changes_and_ignores_sidecar_only_pixel_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watched.bmp");
        image::RgbImage::from_pixel(8, 12, image::Rgb([30, 40, 50]))
            .save(&path)
            .unwrap();
        let mut loader = FolderLoader::new(|| {});
        loader.start(dir.path().into(), false, None, false);
        while !matches!(
            loader.rx.recv_timeout(Duration::from_secs(8)).unwrap(),
            Update::Complete(_)
        ) {}
        image::RgbImage::from_pixel(12, 8, image::Rgb([130, 140, 150]))
            .save(&path)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(Instant::now() < deadline, "image update not delivered");
            if let Update::ImagesChanged(paths) = next_update(&loader, Duration::from_secs(8)) {
                assert!(paths.contains(&path));
                break;
            }
        }
        // Sidecar-only edits do not invalidate expensive decoded image pixels.
        let changes = ChangeInbox::new(Revisions::default(), Arc::new(|| {}));
        changes.record(Ok(notify::Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Any,
        )))
        .add_path(metadata::sidecar(&path))));
        assert!(changes.take().dirty.contains(&path));
        assert!(changes.take_pixels().is_none());
    }
    #[test]
    fn live_watcher_refreshes_sidecar_keywords() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("watched.bmp");
        fs::write(&image, b"image").unwrap();
        let mut loader = FolderLoader::new(|| {});
        loader.start(dir.path().into(), false, None, false);
        while !matches!(
            loader.rx.recv_timeout(Duration::from_secs(8)).unwrap(),
            Update::Complete(_)
        ) {}
        fs::write(metadata::sidecar(&image),b"<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:dc='http://purl.org/dc/elements/1.1/'><dc:subject><rdf:Bag><rdf:li>live</rdf:li></rdf:Bag></dc:subject></rdf:Description></rdf:RDF></x:xmpmeta>").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(Instant::now() < deadline, "sidecar update not delivered");
            if let Update::Keywords(rows) = loader.rx.recv_timeout(Duration::from_secs(8)).unwrap()
            {
                if rows
                    .iter()
                    .any(|r| r.path == image && r.tags.contains("live"))
                {
                    break;
                }
            }
        }
        let side = metadata::sidecar(&image);
        let modified = fs::metadata(&side).unwrap().modified().unwrap();
        let bytes = fs::read_to_string(&side)
            .unwrap()
            .replace(">live<", ">next<");
        fs::write(&side, bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&side)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(
                Instant::now() < deadline,
                "same-size timestamp-preserving update not delivered"
            );
            if let Update::Keywords(rows) = loader.rx.recv_timeout(Duration::from_secs(8)).unwrap()
            {
                if rows
                    .iter()
                    .any(|r| r.path == image && r.tags.contains("next"))
                {
                    break;
                }
            }
        }
        fs::remove_file(metadata::sidecar(&image)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(Instant::now() < deadline, "sidecar removal not delivered");
            if let Update::Keywords(rows) = loader.rx.recv_timeout(Duration::from_secs(8)).unwrap()
            {
                if rows.iter().any(|r| r.path == image && r.tags.is_empty()) {
                    break;
                }
            }
        }
    }
}

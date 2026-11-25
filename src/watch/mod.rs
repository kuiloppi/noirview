//! Background folder watcher: keeps the grid thumbnails and the keyword index
//! of the watched folders up to date, inside the open program or, while it is
//! closed, as a separate `noirview --watch` process (see `control`). One owner
//! at a time holds the control lock, so two writers never share the caches.
pub mod control;
pub mod journal;
pub mod source;

use crate::{
    keyword_index::{self, CachedKeywords, Fingerprint, KeywordIndex},
    model::{Settings, WatchedFolder},
    pregen::Outcome,
    thumbnail_cache::ThumbnailCache,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use source::{CatchUp, Change, ChangeSource, StateDb};
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// A full listing of every folder at most this often (the folder-time check
/// does not see a file edited in place).
const DEEP_EVERY_SECS: u64 = 24 * 60 * 60;
/// Keyword rows are written in batches.
const BATCH: usize = 64;

/// One stretch of work, from the first queued image until the queue is empty
/// (pauses included).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Batch {
    pub images: u64,
    pub thumbnails: u64,
    pub keywords: u64,
    pub secs: u64,
}
impl Batch {
    pub fn describe(&self) -> String {
        let time = if self.secs >= 60 {
            format!("{} min {} s", self.secs / 60, self.secs % 60)
        } else {
            format!("{} s", self.secs)
        };
        let made = self.thumbnails.max(self.keywords);
        let rate = if self.secs > 0 && made > 0 {
            format!(" ({:.1} images/s)", made as f64 / self.secs as f64)
        } else {
            String::new()
        };
        format!(
            "{} image(s) checked, {} thumbnails made, {} keywords read in {time}{rate}",
            self.images, self.thumbnails, self.keywords
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// "starting", "checking", "working", "watching", "paused", "stopped".
    pub state: String,
    /// "NoirView" (inside the open program) or "background".
    pub owner: String,
    pub pid: u32,
    pub version: String,
    pub folders: usize,
    /// Images waiting (live ones and the rest of the current folder).
    pub queue: usize,
    /// Folders waiting to be checked after those.
    pub folders_waiting: usize,
    pub checked: u64,
    pub thumbnails: u64,
    pub keywords: u64,
    pub failed: u64,
    pub cache_full: bool,
    /// The work in progress, and the last finished one.
    pub batch: Option<Batch>,
    pub last_batch: Option<Batch>,
    pub last_check: Option<CatchUp>,
    /// Unix seconds of the last completed check.
    pub last_check_at: u64,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub updated: u64,
}
impl Status {
    pub fn summary(&self) -> String {
        let mut text = format!(
            "{} · {} folder(s) · queue {} image(s) + {} folder(s) · thumbnails made {} · keywords read {}",
            self.state,
            self.folders,
            self.queue,
            self.folders_waiting,
            self.thumbnails,
            self.keywords
        );
        if self.failed > 0 {
            text.push_str(&format!(" · {} unreadable", self.failed));
        }
        if let Some(b) = &self.batch {
            text.push_str(&format!("\nWorking: {} so far", b.describe()));
        }
        if let Some(b) = &self.last_batch {
            text.push_str(&format!("\nLast batch: {}", b.describe()));
        }
        if let Some(c) = &self.last_check {
            text.push_str(&format!(
                "\nLast change check{}: {} folder(s), {} changed, in {} ms",
                if c.deep { " (full)" } else { "" },
                c.folders,
                c.changed,
                c.ms
            ));
            if c.journal > 0 {
                text.push_str(&format!(
                    " · {} watched folder(s) from the change journal",
                    c.journal
                ));
            }
        }
        text
    }
}

/// What the engine needs, independent of where it runs.
pub struct Setup {
    pub folders: Vec<WatchedFolder>,
    pub settings: Settings,
    pub cache: Arc<ThumbnailCache>,
    pub keyword_db: Option<PathBuf>,
    pub state_db: PathBuf,
    pub owner: &'static str,
    /// Background work may run now (the open program yields to the user).
    pub gate: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Additional stop condition (a stop request for the background process).
    pub stop_requested: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// One folder whose images are being checked.
struct FolderWork {
    root: usize,
    dir: PathBuf,
    stamp: String,
    subdirs: Vec<PathBuf>,
    files: Option<VecDeque<PathBuf>>,
}

pub struct Engine {
    setup: Setup,
    source: Box<dyn ChangeSource>,
    state: StateDb,
    index: Option<KeywordIndex>,
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    check_now: Arc<AtomicBool>,
    live: VecDeque<(usize, PathBuf)>,
    queued: HashSet<PathBuf>,
    folders: VecDeque<FolderWork>,
    writes: Vec<Vec<(Vec<u8>, CachedKeywords)>>,
    last_flush: Instant,
    /// Start of the current batch and the counters then.
    batch: Option<(Instant, [u64; 3])>,
}

impl Engine {
    pub fn new(
        setup: Setup,
        source: Box<dyn ChangeSource>,
        status: Arc<Mutex<Status>>,
        stop: Arc<AtomicBool>,
        check_now: Arc<AtomicBool>,
    ) -> Result<Self> {
        let state = StateDb::open(&setup.state_db)?;
        let mut warnings = vec![];
        let index = match setup.keyword_db.as_deref().map(KeywordIndex::open) {
            Some(Ok(index)) => Some(index),
            Some(Err(e)) => {
                warnings.push(format!("Keyword index unavailable: {e:#}"));
                None
            }
            None => None,
        };
        {
            let mut s = status.lock().unwrap();
            *s = Status {
                state: "starting".into(),
                owner: setup.owner.into(),
                pid: std::process::id(),
                version: env!("CARGO_PKG_VERSION").into(),
                folders: setup.folders.len(),
                warnings,
                ..Default::default()
            };
        }
        let roots = setup.folders.len();
        Ok(Self {
            setup,
            source,
            state,
            index,
            status,
            stop,
            check_now,
            live: VecDeque::new(),
            queued: HashSet::new(),
            folders: VecDeque::new(),
            writes: vec![vec![]; roots],
            last_flush: Instant::now(),
            batch: None,
        })
    }
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || (self.setup.stop_requested)()
    }
    fn update(&self, f: impl FnOnce(&mut Status)) {
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut s);
        // Constant time: called after every image.
        let current = self.folders.front().and_then(|w| w.files.as_ref());
        s.queue = self.live.len() + current.map_or(0, VecDeque::len);
        s.folders_waiting = self.folders.len() - usize::from(current.is_some());
        s.updated = source::now_secs();
    }
    /// Catch up on every root (or one after lost notifications).
    /// `daily`: the scheduled full check, skipped for folders the source
    /// does not need it for.
    fn catch_up(&mut self, only: Option<usize>, deep: bool) {
        self.catch_up_with(only, deep, false);
    }
    fn catch_up_with(&mut self, only: Option<usize>, deep: bool, daily: bool) {
        self.update(|s| s.state = "checking".into());
        let mut total = CatchUp {
            deep,
            ..Default::default()
        };
        let mut warnings = vec![];
        for (i, folder) in self.setup.folders.clone().iter().enumerate() {
            if only.is_some_and(|o| o != i) || self.stopping() {
                continue;
            }
            let mut out = vec![];
            let deep = deep && (!daily || self.source.daily_deep(folder, &self.state));
            match self.source.catch_up(i, folder, deep, &self.state, &mut out) {
                Ok(stats) => {
                    total.folders += stats.folders;
                    total.changed += stats.changed;
                    total.ms += stats.ms;
                    total.journal += stats.journal;
                }
                Err(e) => warnings.push(format!("{e:#}")),
            }
            for change in out {
                self.accept(change);
            }
        }
        if deep && only.is_none() {
            let _ = self
                .state
                .set_fact("last_deep", &source::now_secs().to_string());
        }
        warnings.extend(self.source.warnings());
        self.update(|s| {
            s.last_check = Some(total);
            s.last_check_at = source::now_secs();
            s.warnings.retain(|w| !w.contains(" is not available"));
            for w in warnings {
                if !s.warnings.contains(&w) {
                    s.warnings.push(w);
                }
            }
        });
    }
    fn accept(&mut self, change: Change) {
        match change {
            Change::Folder {
                root,
                dir,
                stamp,
                subdirs,
            } => {
                if !self.folders.iter().any(|w| w.dir == dir) {
                    self.folders.push_back(FolderWork {
                        root,
                        dir,
                        stamp,
                        subdirs,
                        files: None,
                    });
                }
            }
            Change::File { root, path } => {
                if self.queued.insert(path.clone()) {
                    self.live.push_back((root, path));
                }
            }
            Change::Removed { root, path } => {
                if let Some(index) = &self.index {
                    let folder = &self.setup.folders[root];
                    let _ = index.remove(&folder.path, folder.subfolders, &path);
                }
            }
            Change::Rescan { root } => self.catch_up(Some(root), false),
        }
    }
    fn deep_due(&self) -> bool {
        let last: u64 = self
            .state
            .fact("last_deep")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        source::now_secs().saturating_sub(last) >= DEEP_EVERY_SECS
    }
    /// Checks one image; returns whether real work (a read) was done.
    fn process(&mut self, root: usize, path: &Path) -> bool {
        if !path.is_file() {
            return false;
        }
        let mut worked = false;
        let mut failed = false;
        if let Some(index) = &self.index {
            let result = (|| -> Result<bool> {
                let stamp = Fingerprint::capture(path)?;
                // A current row from any scope (the viewer's own, too) will do.
                if stamp.reusable() && index.get_any(path, &stamp)?.is_some() {
                    return Ok(false);
                }
                let tags = crate::metadata::read(path)?;
                if Fingerprint::capture(path)? != stamp {
                    return Ok(true);
                }
                self.writes[root].push((keyword_index::key(path), CachedKeywords { stamp, tags }));
                Ok(true)
            })();
            match result {
                Ok(read) => {
                    worked |= read;
                    if read {
                        self.update(|s| s.keywords += 1);
                    }
                }
                Err(_) => failed = true,
            }
        }
        if !self.status.lock().unwrap().cache_full {
            match crate::imaging::generate_thumbnail(&self.setup.cache, path, &self.setup.settings)
            {
                Outcome::Generated => {
                    worked = true;
                    self.update(|s| s.thumbnails += 1);
                }
                Outcome::Full => self.update(|s| s.cache_full = true),
                Outcome::Failed => failed = true,
                Outcome::Cached => {}
            }
        }
        self.update(|s| {
            s.checked += 1;
            s.failed += u64::from(failed);
        });
        worked
    }
    fn flush(&mut self, force: bool) {
        let pending: usize = self.writes.iter().map(Vec::len).sum();
        if pending == 0
            || (!force && pending < BATCH && self.last_flush.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        if let Some(index) = &mut self.index {
            for (root, rows) in self.writes.iter_mut().enumerate() {
                if rows.is_empty() {
                    continue;
                }
                let folder = &self.setup.folders[root];
                if let Err(e) = index.store(&folder.path, folder.subfolders, rows) {
                    let warning = format!("Keyword index could not be saved: {e:#}");
                    let mut s = self.status.lock().unwrap();
                    if !s.warnings.contains(&warning) {
                        s.warnings.push(warning);
                    }
                }
                rows.clear();
            }
            // Written now (this already gathers its rows), so the viewer sees them.
            if let Err(e) = index.write_pending() {
                let warning = format!("Keyword index could not be saved: {e:#}");
                let mut s = self.status.lock().unwrap();
                if !s.warnings.contains(&warning) {
                    s.warnings.push(warning);
                }
            }
        }
        self.last_flush = Instant::now();
    }
    /// The next image to check, or None when idle.
    fn next(&mut self) -> Option<(usize, PathBuf)> {
        if let Some((root, path)) = self.live.pop_front() {
            self.queued.remove(&path);
            return Some((root, path));
        }
        loop {
            let work = self.folders.front_mut()?;
            let files = work.files.get_or_insert_with(|| {
                source::list(&work.dir)
                    .map(|(files, _)| files.into())
                    .unwrap_or_default()
            });
            if let Some(path) = files.pop_front() {
                return Some((work.root, path));
            }
            // Every image of the folder was checked: remember it as done.
            let work = self.folders.pop_front().unwrap();
            self.flush(true);
            let root = &self.setup.folders[work.root];
            let _ = self
                .state
                .set_folder(root, &work.dir, &work.stamp, &work.subdirs);
        }
    }
    fn batch_progress(&self) -> Option<Batch> {
        let (started, [checked, thumbnails, keywords]) = self.batch?;
        let s = self.status.lock().unwrap();
        Some(Batch {
            images: s.checked - checked,
            thumbnails: s.thumbnails - thumbnails,
            keywords: s.keywords - keywords,
            secs: started.elapsed().as_secs(),
        })
    }
    pub fn run(mut self) {
        let deep = self.deep_due();
        self.catch_up_with(None, deep, true);
        loop {
            if self.stopping() {
                break;
            }
            for change in self.source.take_live() {
                self.accept(change);
            }
            if self.check_now.swap(false, Ordering::Relaxed) {
                self.catch_up(None, true);
            } else if self.deep_due() {
                self.catch_up_with(None, true, true);
            }
            let busy = !self.live.is_empty() || !self.folders.is_empty();
            if busy && !(self.setup.gate)() {
                self.update(|s| s.state = "paused".into());
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            if let Some((root, path)) = self.next() {
                if self.batch.is_none() {
                    let s = self.status.lock().unwrap();
                    self.batch = Some((Instant::now(), [s.checked, s.thumbnails, s.keywords]));
                }
                self.update(|s| s.state = "working".into());
                let started = Instant::now();
                // A malformed file must not end the watcher (decoders and
                // metadata readers can panic): it counts as unreadable.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.process(root, &path)
                }));
                match outcome {
                    Ok(true) => std::thread::sleep(crate::pregen::rest_at(
                        started.elapsed(),
                        self.setup.settings.watch_pace,
                    )),
                    Ok(false) => {}
                    Err(_) => {
                        let warning = format!("Could not process {}", path.display());
                        self.update(|s| {
                            s.failed += 1;
                            if s.warnings.len() < 20 {
                                s.warnings.push(warning);
                            }
                        });
                    }
                }
                let progress = self.batch_progress();
                self.update(|s| s.batch = progress);
                self.flush(false);
                continue;
            }
            if let Some(done) = self.batch_progress() {
                self.batch = None;
                self.update(|s| {
                    s.batch = None;
                    s.last_batch = Some(done);
                });
            }
            self.flush(true);
            self.source.idle(&self.state);
            self.update(|s| s.state = "watching".into());
            self.source.wait(Duration::from_millis(500));
        }
        self.flush(true);
        let _ = self.setup.cache.wait_idle(Duration::from_secs(5));
        self.update(|s| s.state = "stopped".into());
    }
}

/// The change journal is used when the setting allows it and the USN
/// service is running (a stopped service: folders are listed, and Settings
/// says the service is not running).
pub fn journal_wanted(settings: &Settings) -> bool {
    settings.watch_use_journal && crate::usn::state() == crate::usn::State::Running
}
/// The change source for the settings: the journal with the folder-time
/// source as its fallback, or the folder-time source alone.
pub fn source_for(
    folders: &[WatchedFolder],
    settings: &Settings,
) -> (Box<dyn ChangeSource>, Vec<String>) {
    if journal_wanted(settings) {
        let source = journal::HybridSource::new(folders, Box::new(journal::PipeJournal));
        let warnings = source.warnings.clone();
        (Box::new(source), warnings)
    } else {
        let source = source::DirectorySource::new(folders);
        let warnings = source.warnings.clone();
        (Box::new(source), warnings)
    }
}
/// A running engine thread.
pub struct Handle {
    pub folders: Vec<WatchedFolder>,
    pub pace: crate::model::WatchPace,
    /// Started with the change-journal source.
    pub journal: bool,
    pub status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    check_now: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Handle {
    pub fn start(setup: Setup, source: Box<dyn ChangeSource>) -> Result<Self> {
        let status = Arc::new(Mutex::new(Status::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let check_now = Arc::new(AtomicBool::new(false));
        let folders = setup.folders.clone();
        let pace = setup.settings.watch_pace;
        let journal = journal_wanted(&setup.settings);
        let engine = Engine::new(
            setup,
            source,
            status.clone(),
            stop.clone(),
            check_now.clone(),
        )?;
        let thread = std::thread::Builder::new()
            .name("folder watcher".into())
            .spawn(move || {
                if !cfg!(test) {
                    crate::sysinfo::enter_background_mode();
                }
                crate::disk::set_background_reader();
                engine.run()
            })?;
        Ok(Self {
            folders,
            pace,
            journal,
            status,
            stop,
            check_now,
            thread: Some(thread),
        })
    }
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn check_now(&self) {
        self.check_now.store(true, Ordering::Relaxed);
    }
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
    /// Stops the engine and waits for it (it finishes the current image).
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use source::DirectorySource;

    fn jpeg(path: &Path, shade: u8) {
        let image = image::RgbImage::from_pixel(64, 48, image::Rgb([shade, 90, 160]));
        image.save(path).unwrap();
    }
    fn wait_for(
        status: &Arc<Mutex<Status>>,
        what: &str,
        until: impl Fn(&Status) -> bool,
    ) -> Status {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let s = status.lock().unwrap().clone();
            if until(&s) {
                return s;
            }
            assert!(Instant::now() < deadline, "{what}: {s:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn setup(tmp: &Path, folders: Vec<WatchedFolder>, cache: Arc<ThumbnailCache>) -> Setup {
        Setup {
            folders,
            settings: Settings::default(),
            cache,
            keyword_db: Some(tmp.join("data/keyword-index.sqlite3")),
            state_db: tmp.join("data/watch-state.sqlite3"),
            owner: "test",
            gate: Arc::new(|| true),
            stop_requested: Arc::new(|| false),
        }
    }

    #[test]
    fn engine_fills_caches_skips_done_work_and_follows_new_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("photos");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        for i in 0..3 {
            jpeg(&root.join(format!("{i}.jpg")), i * 40);
        }
        jpeg(&root.join("sub/deep.jpg"), 200);
        let folders = vec![WatchedFolder {
            path: root.clone(),
            subfolders: true,
        }];
        let cache = Arc::new(ThumbnailCache::new(Some(tmp.path().join("thumbs")), 64));
        let handle = Handle::start(
            setup(tmp.path(), folders.clone(), cache.clone()),
            Box::new(DirectorySource::with_settle(
                &folders,
                Duration::from_millis(200),
            )),
        )
        .unwrap();
        let s = wait_for(&handle.status, "first pass", |s| {
            s.state == "watching" && s.thumbnails == 4
        });
        assert_eq!((s.keywords, s.failed, s.checked), (4, 0, 4));
        let batch = s.last_batch.unwrap();
        assert_eq!((batch.images, batch.thumbnails, batch.keywords), (4, 4, 4));
        assert!(s.batch.is_none());
        assert_eq!(s.last_check.unwrap().changed, 2);
        // Keyword rows are under the watched folder, as the viewer opens it.
        let index = KeywordIndex::open(&tmp.path().join("data/keyword-index.sqlite3")).unwrap();
        assert_eq!(index.load(&root, true).unwrap().len(), 4);
        // A new image is picked up live.
        jpeg(&root.join("new.jpg"), 10);
        wait_for(&handle.status, "live file", |s| {
            s.thumbnails == 5 && s.keywords == 5
        });
        handle.stop();
        cache.flush();
        // A restart lists only the folder that got the live file (`sub` is
        // skipped) and reads nothing again.
        let handle = Handle::start(
            setup(tmp.path(), folders.clone(), cache.clone()),
            Box::new(DirectorySource::with_settle(
                &folders,
                Duration::from_millis(200),
            )),
        )
        .unwrap();
        let s = wait_for(&handle.status, "restart", |s| s.state == "watching");
        assert_eq!((s.thumbnails, s.keywords), (0, 0));
        let check = s.last_check.unwrap();
        assert!(!check.deep);
        assert_eq!((check.folders, check.changed), (2, 1));
        handle.stop();
        // Added while nobody watched: found by the catch-up.
        std::thread::sleep(Duration::from_millis(30));
        jpeg(&root.join("sub/offline.jpg"), 99);
        let handle = Handle::start(
            setup(tmp.path(), folders.clone(), cache.clone()),
            Box::new(DirectorySource::with_settle(
                &folders,
                Duration::from_millis(200),
            )),
        )
        .unwrap();
        let s = wait_for(&handle.status, "offline file", |s| {
            s.state == "watching" && s.thumbnails == 1
        });
        assert_eq!(s.last_check.unwrap().changed, 1);
        // Check now lists everything but makes nothing that exists.
        handle.check_now();
        let s = wait_for(&handle.status, "full check", |s| {
            s.last_check.is_some_and(|c| c.deep) && s.state == "watching"
        });
        assert_eq!((s.thumbnails, s.keywords), (1, 1));
        handle.stop();
    }

    #[test]
    fn a_closed_gate_pauses_the_work_and_stop_ends_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        std::fs::create_dir_all(&root).unwrap();
        jpeg(&root.join("a.jpg"), 1);
        let folders = vec![WatchedFolder {
            path: root,
            subfolders: false,
        }];
        let cache = Arc::new(ThumbnailCache::new(Some(tmp.path().join("thumbs")), 64));
        let open = Arc::new(AtomicBool::new(false));
        let gate = open.clone();
        let mut s = setup(tmp.path(), folders.clone(), cache);
        s.gate = Arc::new(move || gate.load(Ordering::Relaxed));
        let handle = Handle::start(s, Box::new(DirectorySource::new(&folders))).unwrap();
        wait_for(&handle.status, "paused", |s| s.state == "paused");
        assert_eq!(handle.status().thumbnails, 0);
        open.store(true, Ordering::Relaxed);
        wait_for(&handle.status, "resumed", |s| s.thumbnails == 1);
        handle.stop();
    }
}

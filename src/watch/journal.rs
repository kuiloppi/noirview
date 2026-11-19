//! Change detection through the optional USN broker service (NTFS change
//! journal), with the folder-time source as the fallback for everything the
//! journal cannot answer: no service, a network or non-NTFS drive, the first
//! run of a folder, a recreated or wrapped journal, or an error.
//!
//! A position (journal id + USN) is kept per watched folder. It is stored only
//! once the images found up to it have been checked (`idle`), so an
//! interrupted pass is read again. While the watcher idles the position also
//! advances, a minute behind the journal, so the next start reads little.
use super::source::{CatchUp, Change, ChangeSource, DirectorySource, StateDb};
use crate::{
    model::{self, WatchedFolder},
    usn::protocol::{volume_of, Journal, Request, Response},
};
use anyhow::{anyhow, bail, Result};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Talks to the broker (a fake in tests).
pub trait JournalClient: Send {
    fn journal(&self, volume: &str) -> Result<Journal>;
    fn read(&self, request: &Request) -> Result<Response>;
}
/// The real service, over its named pipe.
pub struct PipeJournal;
impl JournalClient for PipeJournal {
    fn journal(&self, volume: &str) -> Result<Journal> {
        match crate::usn::call(&Request::Journal {
            volume: volume.into(),
        })? {
            Response::Journal(j) => Ok(j),
            Response::Error { message } => bail!("{message}"),
            other => bail!("unexpected answer {other:?}"),
        }
    }
    fn read(&self, request: &Request) -> Result<Response> {
        crate::usn::call(request)
    }
}

/// Changes read per request.
const BATCH: usize = 50_000;
/// While idle, the position follows the journal this far behind.
const LAG: Duration = Duration::from_secs(60);

fn position_key(folder: &WatchedFolder) -> String {
    format!(
        "usn:{}:{}",
        if folder.subfolders { '+' } else { '-' },
        folder.path.display()
    )
}
fn parse_position(value: &str) -> Option<(u64, i64)> {
    let (id, usn) = value.split_once(':')?;
    Some((id.parse().ok()?, usn.parse().ok()?))
}

pub struct HybridSource {
    folders: Vec<WatchedFolder>,
    directory: DirectorySource,
    client: Box<dyn JournalClient>,
    /// Positions reached by a catch-up, stored when the queue is empty.
    pending: HashMap<String, (u64, i64)>,
    /// Journal ends seen while idle, stored once they are `LAG` old.
    seen: HashMap<String, (u64, i64, Instant)>,
    last_idle: Option<Instant>,
    lag: Duration,
    pub warnings: Vec<String>,
}

impl HybridSource {
    pub fn new(folders: &[WatchedFolder], client: Box<dyn JournalClient>) -> Self {
        let directory = DirectorySource::new(folders);
        Self::with(folders, directory, client, LAG)
    }
    pub fn with(
        folders: &[WatchedFolder],
        directory: DirectorySource,
        client: Box<dyn JournalClient>,
        lag: Duration,
    ) -> Self {
        let warnings = directory.warnings.clone();
        Self {
            folders: folders.to_vec(),
            directory,
            client,
            pending: HashMap::new(),
            seen: HashMap::new(),
            last_idle: None,
            lag,
            warnings,
        }
    }
    /// The folder's position: a newer one reached in this session, else the
    /// stored one.
    fn position(&self, folder: &WatchedFolder, state: &StateDb) -> Option<(u64, i64)> {
        let key = position_key(folder);
        self.pending
            .get(&key)
            .copied()
            .or_else(|| state.fact(&key).and_then(|v| parse_position(&v)))
    }
    /// Lists folders as before, and remembers where the journal was before
    /// the listing (changes made during it are read next time).
    fn by_folders(
        &mut self,
        root: usize,
        folder: &WatchedFolder,
        deep: bool,
        state: &StateDb,
        out: &mut Vec<Change>,
        volume: Option<&str>,
    ) -> Result<CatchUp> {
        let journal = volume.and_then(|v| self.client.journal(v).ok());
        let stats = self.directory.catch_up(root, folder, deep, state, out)?;
        if let Some(j) = journal {
            self.pending
                .insert(position_key(folder), (j.journal_id, j.next_usn));
        }
        Ok(stats)
    }
    fn by_journal(
        &mut self,
        root: usize,
        folder: &WatchedFolder,
        volume: &str,
        (journal_id, from): (u64, i64),
        state: &StateDb,
        out: &mut Vec<Change>,
    ) -> Result<Option<CatchUp>> {
        let started = Instant::now();
        // The service answers with full long names (`C:\Users\runneradmin`,
        // not `RUNNER~1`): compare in that form, ignoring case.
        let base = canonical(&folder.path);
        let base_lower = base.to_lowercase();
        let mut position = from;
        let mut changed = 0;
        let mut folders_changed = false;
        loop {
            let response = self.client.read(&Request::Read {
                volume: volume.into(),
                journal_id,
                from_usn: position,
                roots: vec![base.clone()],
                max: BATCH,
            })?;
            let (next, changes, more) = match response {
                Response::Read { reset: true, .. } => return Ok(None),
                Response::Read {
                    next_usn,
                    changes,
                    more,
                    ..
                } => (next_usn, changes, more),
                Response::Error { message } => return Err(anyhow!("{message}")),
                other => bail!("unexpected answer {other:?}"),
            };
            for change in changes {
                let path = PathBuf::from(&change.path);
                // Without subfolders only the folder's own files count.
                let parent = path.parent().map(|p| p.to_string_lossy().to_lowercase());
                if !folder.subfolders
                    && parent.as_deref() != Some(base_lower.trim_end_matches('\\'))
                    && !(change.dir && change.path.to_lowercase() == base_lower)
                {
                    continue;
                }
                if change.dir {
                    folders_changed = true;
                    continue;
                }
                let image = sidecar_image(&path).unwrap_or(path);
                if !model::supported(&image) {
                    continue;
                }
                changed += 1;
                out.push(if change.removed && !image.exists() {
                    Change::Removed { root, path: image }
                } else {
                    Change::File { root, path: image }
                });
            }
            position = next;
            if !more {
                break;
            }
        }
        // New, renamed or removed folders: the folder-time check lists
        // exactly those (it skips everything unchanged).
        if folders_changed {
            self.directory.catch_up(root, folder, false, state, out)?;
        }
        self.pending
            .insert(position_key(folder), (journal_id, position));
        Ok(Some(CatchUp {
            deep: false,
            folders: 0,
            changed,
            ms: started.elapsed().as_millis() as u64,
            journal: 1,
        }))
    }
}

/// The folder's full path as Windows reports it (without the `\\?\` prefix).
fn canonical(path: &Path) -> String {
    let text = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
    }
}

fn sidecar_image(path: &Path) -> Option<PathBuf> {
    let is_xmp = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("xmp"));
    is_xmp
        .then(|| path.with_extension(""))
        .filter(|p| model::supported(p))
}

impl ChangeSource for HybridSource {
    fn catch_up(
        &mut self,
        root: usize,
        folder: &WatchedFolder,
        deep: bool,
        state: &StateDb,
        out: &mut Vec<Change>,
    ) -> Result<CatchUp> {
        let drive = volume_of(&folder.path.to_string_lossy());
        let Some(volume) = drive.clone().filter(|_| !deep) else {
            return self.by_folders(root, folder, deep, state, out, drive.as_deref());
        };
        let Some(position) = self.position(folder, state) else {
            // First run of this folder: list it and start the journal here.
            return self.by_folders(root, folder, false, state, out, Some(&volume));
        };
        let mut found = vec![];
        match self.by_journal(root, folder, &volume, position, state, &mut found) {
            Ok(Some(stats)) => {
                out.extend(found);
                Ok(stats)
            }
            // Recreated or wrapped journal: everything is checked once.
            Ok(None) => self.by_folders(root, folder, true, state, out, Some(&volume)),
            Err(e) => {
                let warning =
                    format!("Change journal unavailable ({e:#}); folders are listed instead");
                if !self.warnings.contains(&warning) {
                    self.warnings.push(warning);
                }
                self.directory.catch_up(root, folder, false, state, out)
            }
        }
    }
    fn take_live(&mut self) -> Vec<Change> {
        self.directory.take_live()
    }
    fn wait(&self, timeout: Duration) {
        self.directory.wait(timeout)
    }
    fn daily_deep(&self, folder: &WatchedFolder, state: &StateDb) -> bool {
        // A continuous journal sees edits in place too; folders without a
        // journal position still get the daily full check.
        self.position(folder, state).is_none()
    }
    fn idle(&mut self, state: &StateDb) {
        for (key, position) in self.pending.drain() {
            let _ = state.set_fact(&key, &format!("{}:{}", position.0, position.1));
        }
        // Follow the journal while idle, a minute behind: anything older
        // has had time to be noticed live and checked.
        let every = self.lag.min(Duration::from_secs(10));
        if self.last_idle.is_some_and(|t| t.elapsed() < every) {
            return;
        }
        self.last_idle = Some(Instant::now());
        for folder in self.folders.clone() {
            let key = position_key(&folder);
            let Some(stored) = state.fact(&key).and_then(|v| parse_position(&v)) else {
                continue;
            };
            let Some(volume) = volume_of(&folder.path.to_string_lossy()) else {
                continue;
            };
            let Ok(journal) = self.client.journal(&volume) else {
                continue;
            };
            if journal.journal_id != stored.0 {
                self.seen.remove(&key);
                continue;
            }
            match self.seen.get(&key) {
                Some(&(id, usn, at)) if at.elapsed() >= self.lag => {
                    if id == stored.0 && usn > stored.1 {
                        let _ = state.set_fact(&key, &format!("{id}:{usn}"));
                    }
                    self.seen
                        .insert(key, (journal.journal_id, journal.next_usn, Instant::now()));
                }
                Some(_) => {}
                None => {
                    self.seen
                        .insert(key, (journal.journal_id, journal.next_usn, Instant::now()));
                }
            }
        }
    }
    fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usn::protocol::Change as UsnChange;
    use std::sync::{Arc, Mutex};

    /// A journal whose changes the test sets; counts reads.
    #[derive(Clone, Default)]
    struct Fake {
        inner: Arc<Mutex<FakeState>>,
    }
    #[derive(Default)]
    struct FakeState {
        id: u64,
        next: i64,
        changes: Vec<UsnChange>,
        reset: bool,
        down: bool,
        reads: usize,
    }
    impl JournalClient for Fake {
        fn journal(&self, volume: &str) -> Result<Journal> {
            let s = self.inner.lock().unwrap();
            if s.down {
                bail!("not reachable");
            }
            Ok(Journal {
                volume: volume.into(),
                journal_id: s.id,
                first_usn: 0,
                next_usn: s.next,
                max_size: 32 << 20,
            })
        }
        fn read(&self, request: &Request) -> Result<Response> {
            let mut s = self.inner.lock().unwrap();
            if s.down {
                bail!("not reachable");
            }
            s.reads += 1;
            let Request::Read { journal_id, .. } = request else {
                bail!("unexpected")
            };
            Ok(Response::Read {
                next_usn: s.next,
                changes: std::mem::take(&mut s.changes),
                more: false,
                reset: s.reset || *journal_id != s.id,
            })
        }
    }

    #[test]
    fn positions_parse_and_keys_differ_by_subfolder_setting() {
        assert_eq!(parse_position("7:1024"), Some((7, 1024)));
        assert_eq!(parse_position("x"), None);
        let a = WatchedFolder {
            path: "D:\\P".into(),
            subfolders: true,
        };
        let b = WatchedFolder {
            subfolders: false,
            ..a.clone()
        };
        assert_ne!(position_key(&a), position_key(&b));
    }

    /// The journal logic on a drive-letter path, with the folder listing
    /// replaced by the fake's answers (works on every platform).
    #[test]
    fn journal_changes_map_to_images_and_positions_wait_for_idle() {
        let tmp = tempfile::tempdir().unwrap();
        let state = StateDb::open(&tmp.path().join("state.sqlite3")).unwrap();
        let folder = WatchedFolder {
            path: PathBuf::from(r"Q:\Photos"),
            subfolders: true,
        };
        let fake = Fake::default();
        {
            let mut s = fake.inner.lock().unwrap();
            s.id = 7;
            s.next = 1000;
        }
        let mut source = HybridSource::with(
            std::slice::from_ref(&folder),
            DirectorySource::new(&[]),
            Box::new(fake.clone()),
            Duration::from_millis(30),
        );
        // A stored position: the journal is read instead of listing.
        state.set_fact(&position_key(&folder), "7:500").unwrap();
        {
            let mut s = fake.inner.lock().unwrap();
            s.next = 1200;
            s.changes = vec![
                UsnChange {
                    path: r"Q:\Photos\new.jpg".into(),
                    reasons: 0x100,
                    dir: false,
                    removed: false,
                },
                UsnChange {
                    path: r"Q:\Photos\notes.txt".into(),
                    reasons: 0x100,
                    dir: false,
                    removed: false,
                },
                UsnChange {
                    path: r"Q:\Photos\gone.png".into(),
                    reasons: 0x200,
                    dir: false,
                    removed: true,
                },
                UsnChange {
                    path: r"Q:\Photos\old.jpg.xmp".into(),
                    reasons: 0x1,
                    dir: false,
                    removed: false,
                },
            ];
        }
        let mut out = vec![];
        let stats = source
            .catch_up(0, &folder, false, &state, &mut out)
            .unwrap();
        assert_eq!((stats.journal, stats.changed, stats.folders), (1, 3, 0));
        assert!(out.contains(&Change::File {
            root: 0,
            path: PathBuf::from(r"Q:\Photos\new.jpg")
        }));
        assert!(out.contains(&Change::Removed {
            root: 0,
            path: PathBuf::from(r"Q:\Photos\gone.png")
        }));
        assert!(out.contains(&Change::File {
            root: 0,
            path: PathBuf::from(r"Q:\Photos\old.jpg")
        }));
        // Not stored until the images were checked (the engine idles).
        assert_eq!(state.fact(&position_key(&folder)).as_deref(), Some("7:500"));
        assert!(!source.daily_deep(&folder, &state));
        source.idle(&state);
        assert_eq!(
            state.fact(&position_key(&folder)).as_deref(),
            Some("7:1200")
        );
        // Idle follows the journal a lag behind: an end seen now is stored
        // once it is older than the lag.
        fake.inner.lock().unwrap().next = 1500;
        source.idle(&state);
        std::thread::sleep(Duration::from_millis(60));
        source.idle(&state);
        assert_eq!(
            state.fact(&position_key(&folder)).as_deref(),
            Some("7:1200")
        );
        std::thread::sleep(Duration::from_millis(60));
        source.idle(&state);
        assert_eq!(
            state.fact(&position_key(&folder)).as_deref(),
            Some("7:1500")
        );
        // A recreated journal: the folder is checked in full (the listing of
        // a missing Q: drive fails, which is reported).
        fake.inner.lock().unwrap().id = 8;
        let mut out = vec![];
        assert!(source
            .catch_up(0, &folder, false, &state, &mut out)
            .is_err());
        // Service gone: a warning, and the listing is used.
        fake.inner.lock().unwrap().down = true;
        state.set_fact(&position_key(&folder), "8:1").unwrap();
        let _ = source.catch_up(0, &folder, false, &state, &mut vec![]);
        assert!(source
            .warnings()
            .iter()
            .any(|w| w.contains("Change journal unavailable")));
    }
}

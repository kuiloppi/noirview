//! Where the watcher learns about changes. `DirectorySource` works on any
//! file system without extra rights: a catch-up pass compares each folder's
//! modification time with the stored one (a folder's time changes whenever a
//! file is created, deleted or renamed in it, so unchanged folders are not
//! listed at all), and live notifications cover the time in between. A USN
//! journal source can implement the same trait later.
use crate::model::{self, WatchedFolder};
use anyhow::{Context, Result};
use notify::{event::ModifyKind, EventKind, RecursiveMode, Watcher};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Something the engine must look at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A folder whose images must be checked; its stamp and subfolders are
    /// stored only after that, so an interrupted pass is repeated.
    Folder {
        root: usize,
        dir: PathBuf,
        stamp: String,
        subdirs: Vec<PathBuf>,
    },
    File {
        root: usize,
        path: PathBuf,
    },
    Removed {
        root: usize,
        path: PathBuf,
    },
    /// Live notifications were lost: catch up this root again.
    Rescan {
        root: usize,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CatchUp {
    pub deep: bool,
    pub folders: usize,
    pub changed: usize,
    pub ms: u64,
    /// Watched folders answered from the NTFS change journal.
    pub journal: usize,
}

pub trait ChangeSource: Send {
    /// Folders (or files) that changed since the stored state; all of them
    /// with `deep`.
    fn catch_up(
        &mut self,
        root: usize,
        folder: &WatchedFolder,
        deep: bool,
        state: &StateDb,
        out: &mut Vec<Change>,
    ) -> Result<CatchUp>;
    /// Live changes that have settled (a file being copied is reported once
    /// it has been quiet for a moment).
    fn take_live(&mut self) -> Vec<Change>;
    /// Sleep until a live change may be due, at most `timeout`.
    fn wait(&self, timeout: Duration);
    /// Whether the daily full check is needed for this folder (a continuous
    /// change journal sees in-place edits, folder times do not).
    fn daily_deep(&self, _folder: &WatchedFolder, _state: &StateDb) -> bool {
        true
    }
    /// The queue is empty: everything found so far has been checked.
    fn idle(&mut self, _state: &StateDb) {}
    fn warnings(&self) -> Vec<String> {
        vec![]
    }
}

/// Folder stamps and subfolders from the last completed pass, plus small
/// key/value facts (time of the last deep pass).
pub struct StateDb {
    connection: Connection,
}
fn key(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}
/// State is kept per watched folder and subfolder setting: turning
/// subfolders on must not reuse a pass that never looked at them.
fn root_key(folder: &WatchedFolder) -> Vec<u8> {
    let mut key = key(&folder.path);
    key.extend_from_slice(if folder.subfolders { b"\0+" } else { b"\0-" });
    key
}
fn path_of(bytes: Vec<u8>) -> PathBuf {
    // SAFETY: the bytes were produced by `as_encoded_bytes` on this platform.
    unsafe { std::ffi::OsString::from_encoded_bytes_unchecked(bytes) }.into()
}
impl StateDb {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path).context("Cannot open the watcher state")?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
            CREATE TABLE IF NOT EXISTS folders (
                root BLOB NOT NULL, path BLOB NOT NULL, stamp TEXT NOT NULL, subdirs TEXT NOT NULL,
                PRIMARY KEY(root, path)) WITHOUT ROWID;
            CREATE TABLE IF NOT EXISTS facts (name TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Ok(Self { connection })
    }
    pub fn folder(
        &self,
        root: &WatchedFolder,
        dir: &Path,
    ) -> Result<Option<(String, Vec<PathBuf>)>> {
        let row = self
            .connection
            .prepare_cached("SELECT stamp, subdirs FROM folders WHERE root=?1 AND path=?2")?
            .query_row(params![root_key(root), key(dir)], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .optional()?;
        row.map(|(stamp, subdirs)| {
            let subdirs: Vec<Vec<u8>> = serde_json::from_str(&subdirs)?;
            Ok((stamp, subdirs.into_iter().map(path_of).collect()))
        })
        .transpose()
    }
    pub fn set_folder(
        &self,
        root: &WatchedFolder,
        dir: &Path,
        stamp: &str,
        subdirs: &[PathBuf],
    ) -> Result<()> {
        let subdirs: Vec<Vec<u8>> = subdirs.iter().map(|p| key(p)).collect();
        self.connection.execute(
            "INSERT INTO folders(root,path,stamp,subdirs) VALUES(?1,?2,?3,?4)
             ON CONFLICT(root,path) DO UPDATE SET stamp=excluded.stamp, subdirs=excluded.subdirs",
            params![
                root_key(root),
                key(dir),
                stamp,
                serde_json::to_string(&subdirs)?
            ],
        )?;
        Ok(())
    }
    /// Forget folders of a root that no longer exist.
    pub fn retain_folders(&self, root: &WatchedFolder, keep: &HashSet<PathBuf>) -> Result<()> {
        let mut query = self
            .connection
            .prepare_cached("SELECT path FROM folders WHERE root=?1")?;
        let stale: Vec<Vec<u8>> = query
            .query_map(params![root_key(root)], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<Vec<u8>>>>()?
            .into_iter()
            .filter(|p| !keep.contains(&path_of(p.clone())))
            .collect();
        for path in stale {
            self.connection.execute(
                "DELETE FROM folders WHERE root=?1 AND path=?2",
                params![root_key(root), path],
            )?;
        }
        Ok(())
    }
    pub fn fact(&self, name: &str) -> Option<String> {
        self.connection
            .query_row("SELECT value FROM facts WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .ok()
    }
    pub fn set_fact(&self, name: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO facts(name,value) VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET value=excluded.value",
            params![name, value],
        )?;
        Ok(())
    }
}

/// A folder's modification time, the part of its state that changes when
/// entries are added, removed or renamed.
pub fn folder_stamp(dir: &Path) -> Option<String> {
    let modified = std::fs::metadata(dir).ok()?.modified().ok()?;
    Some(match modified.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos().to_string(),
        Err(e) => format!("-{}", e.duration().as_nanos()),
    })
}
/// Supported images and subfolders directly in `dir`.
pub fn list(dir: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut files = vec![];
    let mut dirs = vec![];
    for entry in std::fs::read_dir(dir).with_context(|| format!("Cannot list {}", dir.display()))? {
        let Ok(entry) = entry else { continue };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            dirs.push(path);
        } else if kind.is_file() && model::supported(&path) {
            files.push(path);
        }
    }
    files.sort();
    dirs.sort();
    Ok((files, dirs))
}

/// Live notifications waiting to settle.
#[derive(Default)]
struct Inbox {
    files: HashMap<PathBuf, Instant>,
    removed: HashSet<PathBuf>,
    rescan: HashSet<usize>,
}
/// A file is handed over once no event has touched it for this long, so a
/// copy in progress is not read half-written.
pub const SETTLE: Duration = Duration::from_secs(2);

pub struct DirectorySource {
    roots: Vec<WatchedFolder>,
    inbox: Arc<(Mutex<Inbox>, Condvar)>,
    _watchers: Vec<notify::RecommendedWatcher>,
    pub warnings: Vec<String>,
    settle: Duration,
}
fn root_of(roots: &[WatchedFolder], path: &Path) -> Option<usize> {
    roots
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            path.starts_with(&r.path)
                && (r.subfolders || path.parent() == Some(r.path.as_path()) || path == r.path)
        })
        .max_by_key(|(_, r)| r.path.as_os_str().len())
        .map(|(i, _)| i)
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
impl DirectorySource {
    pub fn new(roots: &[WatchedFolder]) -> Self {
        Self::with_settle(roots, SETTLE)
    }
    pub fn with_settle(roots: &[WatchedFolder], settle: Duration) -> Self {
        let inbox: Arc<(Mutex<Inbox>, Condvar)> = Arc::default();
        let mut watchers = vec![];
        let mut warnings = vec![];
        for (i, root) in roots.iter().enumerate() {
            let shared = inbox.clone();
            let all = roots.to_vec();
            let watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    let (lock, wake) = &*shared;
                    let mut inbox = lock.lock().unwrap();
                    let event = match event {
                        Ok(event) => event,
                        Err(_) => {
                            inbox.rescan.insert(i);
                            wake.notify_all();
                            return;
                        }
                    };
                    if event.need_rescan() {
                        inbox.rescan.insert(i);
                        wake.notify_all();
                        return;
                    }
                    if event.kind.is_access()
                        || matches!(event.kind, EventKind::Modify(ModifyKind::Metadata(_)))
                    {
                        return;
                    }
                    let removed = matches!(
                        event.kind,
                        EventKind::Remove(_)
                            | EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::From))
                    );
                    for path in &event.paths {
                        let path = sidecar_image(path).unwrap_or_else(|| path.clone());
                        if model::supported(&path) {
                            if root_of(&all, &path).is_none() {
                                continue;
                            }
                            if removed && !path.exists() {
                                inbox.files.remove(&path);
                                inbox.removed.insert(path);
                            } else {
                                inbox.removed.remove(&path);
                                inbox.files.insert(path, Instant::now());
                            }
                        } else if path.is_dir()
                            || matches!(
                                event.kind,
                                EventKind::Create(notify::event::CreateKind::Folder)
                            )
                            || (removed && path.extension().is_none())
                        {
                            // A folder appeared, left or was renamed: its images
                            // are found by catching up on the root.
                            if let Some(r) = root_of(&all, &path) {
                                inbox.rescan.insert(r);
                            }
                        }
                    }
                    wake.notify_all();
                });
            let mode = if root.subfolders {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            match watcher.and_then(|mut w| w.watch(&root.path, mode).map(|()| w)) {
                Ok(w) => watchers.push(w),
                Err(e) => warnings.push(format!(
                    "Live updates unavailable for {}: {e}",
                    root.path.display()
                )),
            }
        }
        Self {
            roots: roots.to_vec(),
            inbox,
            _watchers: watchers,
            warnings,
            settle,
        }
    }
}
impl ChangeSource for DirectorySource {
    fn catch_up(
        &mut self,
        root: usize,
        folder: &WatchedFolder,
        deep: bool,
        state: &StateDb,
        out: &mut Vec<Change>,
    ) -> Result<CatchUp> {
        let started = Instant::now();
        let mut stats = CatchUp {
            deep,
            ..Default::default()
        };
        let mut queue = vec![folder.path.clone()];
        let mut seen = HashSet::new();
        while let Some(dir) = queue.pop() {
            if !seen.insert(dir.clone()) {
                continue;
            }
            // Read before listing: anything added later changes it again.
            let Some(stamp) = folder_stamp(&dir) else {
                if dir == folder.path {
                    anyhow::bail!("{} is not available", dir.display());
                }
                continue;
            };
            stats.folders += 1;
            let known = state.folder(folder, &dir)?;
            let subdirs = match known {
                Some((old, subdirs)) if !deep && old == stamp => subdirs,
                _ => {
                    let (_, subdirs) = match list(&dir) {
                        Ok(listing) => listing,
                        Err(_) if dir != folder.path => continue,
                        Err(e) => return Err(e),
                    };
                    let subdirs = if folder.subfolders { subdirs } else { vec![] };
                    stats.changed += 1;
                    out.push(Change::Folder {
                        root,
                        dir: dir.clone(),
                        stamp,
                        subdirs: subdirs.clone(),
                    });
                    subdirs
                }
            };
            if folder.subfolders {
                queue.extend(subdirs.into_iter().rev());
            }
        }
        state.retain_folders(folder, &seen)?;
        stats.ms = started.elapsed().as_millis() as u64;
        Ok(stats)
    }
    fn take_live(&mut self) -> Vec<Change> {
        let (lock, _) = &*self.inbox;
        let mut inbox = lock.lock().unwrap();
        let mut out: Vec<Change> = inbox
            .rescan
            .drain()
            .map(|root| Change::Rescan { root })
            .collect();
        for path in inbox.removed.drain() {
            if let Some(root) = root_of(&self.roots, &path) {
                out.push(Change::Removed { root, path });
            }
        }
        let settled: Vec<PathBuf> = inbox
            .files
            .iter()
            .filter(|(_, at)| at.elapsed() >= self.settle)
            .map(|(p, _)| p.clone())
            .collect();
        for path in settled {
            inbox.files.remove(&path);
            if let Some(root) = root_of(&self.roots, &path) {
                out.push(Change::File { root, path });
            }
        }
        out
    }
    fn wait(&self, timeout: Duration) {
        let (lock, wake) = &*self.inbox;
        let inbox = lock.lock().unwrap();
        let due = inbox
            .files
            .values()
            .map(|at| self.settle.saturating_sub(at.elapsed()))
            .min();
        if !inbox.rescan.is_empty() || !inbox.removed.is_empty() || due == Some(Duration::ZERO) {
            return;
        }
        let timeout = due.map_or(timeout, |d| d.min(timeout));
        let _ = wake.wait_timeout(inbox, timeout);
    }
}

/// Seconds since the Unix epoch (status and fact timestamps).
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &Path, subfolders: bool) -> WatchedFolder {
        WatchedFolder {
            path: path.into(),
            subfolders,
        }
    }
    fn complete(state: &StateDb, root: &WatchedFolder, changes: &[Change]) {
        for change in changes {
            if let Change::Folder {
                dir,
                stamp,
                subdirs,
                ..
            } = change
            {
                state.set_folder(root, dir, stamp, subdirs).unwrap();
            }
        }
    }
    fn dirs(changes: &[Change]) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = changes
            .iter()
            .filter_map(|c| match c {
                Change::Folder { dir, .. } => Some(dir.clone()),
                _ => None,
            })
            .collect();
        out.sort();
        out
    }
    /// Folder times have coarse resolution on some file systems.
    fn tick() {
        std::thread::sleep(Duration::from_millis(30));
    }

    #[test]
    fn catch_up_lists_only_folders_whose_entries_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("photos");
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("1.jpg"), b"x").unwrap();
        let state = StateDb::open(&tmp.path().join("state.sqlite3")).unwrap();
        let mut source = DirectorySource::new(&[]);
        let watched = folder(&root, true);
        let mut out = vec![];
        let first = source
            .catch_up(0, &watched, false, &state, &mut out)
            .unwrap();
        assert_eq!((first.folders, first.changed), (3, 3));
        // Nothing stored yet (as if interrupted): the pass repeats.
        let mut again = vec![];
        source
            .catch_up(0, &watched, false, &state, &mut again)
            .unwrap();
        assert_eq!(dirs(&again), dirs(&out));
        complete(&state, &watched, &out);
        let mut quiet = vec![];
        let stats = source
            .catch_up(0, &watched, false, &state, &mut quiet)
            .unwrap();
        assert_eq!((stats.folders, stats.changed), (3, 0));
        // A new file changes only its own folder; deep lists everything.
        tick();
        std::fs::write(b.join("2.png"), b"x").unwrap();
        let mut one = vec![];
        source
            .catch_up(0, &watched, false, &state, &mut one)
            .unwrap();
        assert_eq!(dirs(&one), vec![b.clone()]);
        let mut deep = vec![];
        let stats = source
            .catch_up(0, &watched, true, &state, &mut deep)
            .unwrap();
        assert_eq!((stats.deep, stats.changed), (true, 3));
        // A new subfolder is found through its changed parent.
        complete(&state, &watched, &deep);
        tick();
        let c = a.join("c");
        std::fs::create_dir(&c).unwrap();
        let mut nested = vec![];
        source
            .catch_up(0, &watched, false, &state, &mut nested)
            .unwrap();
        assert_eq!(dirs(&nested), vec![a.clone(), c.clone()]);
        // Without subfolders only the root is looked at.
        let flat = folder(&root, false);
        let mut top = vec![];
        let stats = source.catch_up(1, &flat, false, &state, &mut top).unwrap();
        assert_eq!((stats.folders, dirs(&top)), (1, vec![root.clone()]));
    }

    #[test]
    fn live_files_settle_before_they_are_handed_over() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("in");
        std::fs::create_dir_all(&root).unwrap();
        let mut source =
            DirectorySource::with_settle(&[folder(&root, true)], Duration::from_millis(300));
        assert!(source.warnings.is_empty(), "{:?}", source.warnings);
        let path = root.join("new.jpg");
        std::fs::write(&path, b"partial").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = vec![];
        while got.is_empty() && Instant::now() < deadline {
            source.wait(Duration::from_millis(100));
            got = source.take_live();
        }
        assert!(
            got.contains(&Change::File {
                root: 0,
                path: path.clone()
            }),
            "{got:?}"
        );
        std::fs::remove_file(&path).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut removed = vec![];
        while !removed.contains(&Change::Removed {
            root: 0,
            path: path.clone(),
        }) && Instant::now() < deadline
        {
            source.wait(Duration::from_millis(100));
            removed.extend(source.take_live());
        }
        assert!(
            removed.contains(&Change::Removed { root: 0, path }),
            "{removed:?}"
        );
    }

    #[test]
    fn events_map_to_the_most_specific_watched_root() {
        let roots = [
            folder(Path::new("/p"), true),
            folder(Path::new("/p/inner"), false),
        ];
        assert_eq!(root_of(&roots, Path::new("/p/x/1.jpg")), Some(0));
        assert_eq!(root_of(&roots, Path::new("/p/inner/1.jpg")), Some(1));
        assert_eq!(root_of(&roots, Path::new("/p/inner/deep/1.jpg")), Some(0));
        assert_eq!(root_of(&roots, Path::new("/q/1.jpg")), None);
        assert_eq!(
            sidecar_image(Path::new("/p/1.jpg.xmp")),
            Some(PathBuf::from("/p/1.jpg"))
        );
    }
}

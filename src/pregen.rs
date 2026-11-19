//! Background thumbnail pre-generation for the open folder. Workers take the
//! next path only while the viewer is otherwise idle, skip entries already in
//! the disk cache and stop when the cache budget is nearly used, so a pass
//! never evicts its own earlier results.
use crate::model::Settings;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Cached,
    Generated,
    Failed,
    /// The disk cache is (nearly) full; the pass stops here.
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub generated: usize,
    pub full: bool,
}

#[derive(Default)]
struct State {
    paths: Vec<PathBuf>,
    next: usize,
    progress: Progress,
    list: u64,
    settings: Settings,
    enabled: bool,
    stopped: bool,
}

pub struct Pregen {
    state: Mutex<State>,
    wake: Condvar,
}

const IDLE_POLL: Duration = Duration::from_millis(50);
/// No input for this long before the pass may continue.
const QUIET_AFTER_INPUT_MS: u64 = 2000;
/// Set by the UI: window unfocused, keyword indexing running or memory high.
static BLOCKED: AtomicBool = AtomicBool::new(false);
/// Milliseconds (process clock, +1) of the latest user input; 0 = none yet.
static LAST_INPUT: AtomicU64 = AtomicU64::new(0);
/// The folder watcher inside the open program has gates of its own: input
/// is always noted here, and its settings decide what pauses it.
static INPUT_ANY: AtomicU64 = AtomicU64::new(0);
static FOCUSED: AtomicBool = AtomicBool::new(true);
static BUSY: AtomicBool = AtomicBool::new(false);
static WATCH_WHILE_ACTIVE: AtomicBool = AtomicBool::new(false);
static WATCH_WHEN_UNFOCUSED: AtomicBool = AtomicBool::new(true);
/// Called once per frame by the viewer.
pub fn note_frame(input: bool, focused: bool, busy: bool, while_active: bool, unfocused: bool) {
    if input {
        INPUT_ANY.store(clock_ms(), Ordering::Relaxed);
    }
    FOCUSED.store(focused, Ordering::Relaxed);
    BUSY.store(busy, Ordering::Relaxed);
    WATCH_WHILE_ACTIVE.store(while_active, Ordering::Relaxed);
    WATCH_WHEN_UNFOCUSED.store(unfocused, Ordering::Relaxed);
}
/// The folder watcher may work: no keyword indexing or memory pressure, and
/// the user is not active / the window is not in front (unless allowed).
pub fn watcher_quiet() -> bool {
    watcher_quiet_at(clock_ms())
}
fn watcher_quiet_at(now: u64) -> bool {
    let last = INPUT_ANY.load(Ordering::Relaxed);
    let still = last == 0 || now.saturating_sub(last) >= QUIET_AFTER_INPUT_MS;
    let focused = FOCUSED.load(Ordering::Relaxed);
    !BUSY.load(Ordering::Relaxed)
        && if focused {
            still || WATCH_WHILE_ACTIVE.load(Ordering::Relaxed)
        } else {
            WATCH_WHEN_UNFOCUSED.load(Ordering::Relaxed)
        }
}
/// Serializes tests that change the process-wide gates.
#[cfg(test)]
pub(crate) static GATE_TEST: Mutex<()> = Mutex::new(());
fn clock_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}
/// The user moved the mouse, pressed a key or scrolled.
pub fn note_activity() {
    LAST_INPUT.store(clock_ms(), Ordering::Relaxed);
}
pub fn set_blocked(blocked: bool) {
    BLOCKED.store(blocked, Ordering::Relaxed);
}
/// Background work may run: nothing blocks it and the user has been still.
pub fn quiet() -> bool {
    quiet_at(clock_ms())
}
fn quiet_at(now: u64) -> bool {
    let last = LAST_INPUT.load(Ordering::Relaxed);
    !BLOCKED.load(Ordering::Relaxed)
        && (last == 0 || now.saturating_sub(last) >= QUIET_AFTER_INPUT_MS)
}
/// Rest after generating one thumbnail: as long as the work took (half duty),
/// so a hard disk always has room for other requests.
pub fn rest(worked: Duration) -> Duration {
    if cfg!(test) {
        return Duration::ZERO;
    }
    worked.clamp(Duration::from_millis(15), Duration::from_millis(400))
}

/// The folder watcher's rest after one image, by its pace setting.
pub fn rest_at(worked: Duration, pace: crate::model::WatchPace) -> Duration {
    use crate::model::WatchPace;
    if cfg!(test) {
        return Duration::ZERO;
    }
    match pace {
        WatchPace::Gentle => rest(worked),
        WatchPace::Balanced => {
            (worked / 3).clamp(Duration::from_millis(5), Duration::from_millis(150))
        }
        WatchPace::Fast => Duration::ZERO,
    }
}

impl Pregen {
    pub fn start(
        workers: usize,
        idle: impl Fn() -> bool + Send + Sync + 'static,
        work: impl Fn(&Path, &Settings) -> Outcome + Send + Sync + 'static,
    ) -> Arc<Self> {
        let pregen = Arc::new(Self {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let idle = Arc::new(idle);
        let work = Arc::new(work);
        for _ in 0..workers.max(1) {
            let (pregen, idle, work) = (pregen.clone(), idle.clone(), work.clone());
            std::thread::spawn(move || {
                // Lowest CPU, disk and memory priority; never waits in the
                // foreground readers' disk turn.
                // (Unit tests keep normal priority: a loaded test machine
                // would starve a background-mode thread past their time limits.)
                if !cfg!(test) {
                    crate::sysinfo::enter_background_mode();
                }
                crate::disk::set_background_reader();
                pregen.run(&*idle, &*work)
            });
        }
        pregen
    }
    fn pending(s: &State) -> bool {
        s.enabled && !s.progress.full && s.next < s.paths.len()
    }
    fn run(&self, idle: &dyn Fn() -> bool, work: &dyn Fn(&Path, &Settings) -> Outcome) {
        loop {
            // Wait for work, then for the viewer to be idle.
            loop {
                let s = self.state.lock().unwrap();
                if s.stopped {
                    return;
                }
                if !Self::pending(&s) {
                    drop(self.wake.wait(s).unwrap());
                    continue;
                }
                drop(s);
                if idle() {
                    break;
                }
                std::thread::sleep(IDLE_POLL);
            }
            let (path, settings, list) = {
                let mut s = self.state.lock().unwrap();
                if s.stopped {
                    return;
                }
                if !Self::pending(&s) {
                    continue;
                }
                let path = s.paths[s.next].clone();
                s.next += 1;
                (path, s.settings.clone(), s.list)
            };
            let started = Instant::now();
            let outcome = work(&path, &settings);
            if outcome == Outcome::Generated {
                std::thread::sleep(rest(started.elapsed()));
            }
            let mut s = self.state.lock().unwrap();
            if s.list == list {
                match outcome {
                    Outcome::Full => s.progress.full = true,
                    Outcome::Generated => {
                        s.progress.done += 1;
                        s.progress.generated += 1;
                    }
                    Outcome::Cached | Outcome::Failed => s.progress.done += 1,
                }
            }
        }
    }
    /// Replace the list (already ordered from the current page onward).
    pub fn set(&self, paths: Vec<PathBuf>, settings: &Settings, enabled: bool) {
        let mut s = self.state.lock().unwrap();
        s.list += 1;
        s.progress = Progress {
            total: paths.len(),
            ..Default::default()
        };
        s.paths = paths;
        s.next = 0;
        s.settings = settings.clone();
        s.enabled = enabled;
        self.wake.notify_all();
    }
    pub fn progress(&self) -> Option<Progress> {
        let s = self.state.lock().unwrap();
        (s.enabled && s.progress.total > 0).then_some(s.progress)
    }
    pub fn stop(&self) {
        self.state.lock().unwrap().stopped = true;
        self.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        sync::atomic::{AtomicBool, Ordering},
        time::Instant,
    };

    fn wait(pregen: &Pregen, until: impl Fn(Progress) -> bool) -> Progress {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let p = pregen.progress().unwrap_or_default();
            if until(p) {
                return p;
            }
            assert!(Instant::now() < deadline, "pre-generation stalled: {p:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn generates_missing_skips_cached_and_waits_for_idle() {
        let cached: HashSet<PathBuf> = ["1", "3"].iter().map(PathBuf::from).collect();
        let idle = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (i, w) = (idle.clone(), seen.clone());
        let pregen = Pregen::start(
            3,
            move || i.load(Ordering::Relaxed),
            move |path, _| {
                w.lock().unwrap().push(path.to_path_buf());
                if cached.contains(path) {
                    Outcome::Cached
                } else {
                    Outcome::Generated
                }
            },
        );
        let paths: Vec<PathBuf> = (0..6).map(|n| PathBuf::from(n.to_string())).collect();
        pregen.set(paths.clone(), &Settings::default(), true);
        std::thread::sleep(Duration::from_millis(150));
        assert!(seen.lock().unwrap().is_empty(), "must wait for idle");
        idle.store(true, Ordering::Relaxed);
        let p = wait(&pregen, |p| p.done == 6);
        assert_eq!(p.generated, 4);
        let mut got = seen.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, paths);
        pregen.stop();
    }

    #[test]
    fn full_cache_stops_and_a_new_list_restarts_counting() {
        let pregen = Pregen::start(
            1,
            || true,
            |path, _| {
                if path == Path::new("stop") {
                    Outcome::Full
                } else {
                    Outcome::Generated
                }
            },
        );
        let list = ["a", "b", "stop", "c", "d"].map(PathBuf::from).to_vec();
        pregen.set(list, &Settings::default(), true);
        let p = wait(&pregen, |p| p.full);
        assert_eq!((p.done, p.total), (2, 5));
        pregen.set(vec![PathBuf::from("x")], &Settings::default(), true);
        let p = wait(&pregen, |p| p.done == 1);
        assert_eq!((p.total, p.full), (1, false));
        pregen.set(vec![PathBuf::from("y")], &Settings::default(), false);
        assert_eq!(pregen.progress(), None);
        pregen.stop();
    }
    #[test]
    fn watcher_gates_follow_their_own_settings() {
        let _gates = GATE_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let at = |input: bool, focused: bool, active: bool, unfocused: bool| {
            INPUT_ANY.store(0, Ordering::Relaxed);
            note_frame(input, focused, false, active, unfocused);
            watcher_quiet_at(clock_ms() + 10)
        };
        // Focused: pauses during input unless allowed while active.
        assert!(at(false, true, false, true));
        assert!(!at(true, true, false, true));
        assert!(at(true, true, true, true));
        // Another program in front: works only when allowed.
        assert!(at(false, false, false, true));
        assert!(!at(false, false, false, false));
        // Keyword indexing or memory pressure always pauses it.
        note_frame(false, true, true, true, true);
        assert!(!watcher_quiet());
        note_frame(false, true, false, false, true);
        INPUT_ANY.store(0, Ordering::Relaxed);
    }
    #[test]
    fn input_and_blocking_pause_the_pass_until_the_user_has_been_still() {
        let _gates = GATE_TEST.lock().unwrap_or_else(|e| e.into_inner());
        set_blocked(false);
        LAST_INPUT.store(0, Ordering::Relaxed);
        assert!(quiet_at(5));
        LAST_INPUT.store(1000, Ordering::Relaxed);
        assert!(!quiet_at(1000 + QUIET_AFTER_INPUT_MS - 1));
        assert!(quiet_at(1000 + QUIET_AFTER_INPUT_MS));
        set_blocked(true);
        assert!(!quiet_at(1_000_000));
        set_blocked(false);
        LAST_INPUT.store(0, Ordering::Relaxed);
        assert!(quiet());
    }
}

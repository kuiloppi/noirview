//! Disk access shared by background readers. Thumbnail sources are read whole,
//! one file at a time, and decoded in parallel afterwards: on a hard disk,
//! several threads reading different files at once mostly add head seeks.
//! Keyword indexing briefly yields (≤200 ms per file) while visible grid tiles wait.
use std::{
    fs,
    io::{self, Read},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Condvar, Mutex,
    },
    time::{Duration, Instant},
};

static READ: Mutex<()> = Mutex::new(());
/// Visible grid thumbnails that are not on screen yet.
static GRID_DEMAND: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static BACKGROUND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Mark the calling thread as a low-priority background reader. Its slow,
/// throttled reads must not hold the turn that visible thumbnails wait for.
pub fn set_background_reader() {
    BACKGROUND.with(|b| b.set(true));
}

/// Read a whole file (at most `limit` bytes) while holding the shared read
/// turn (background readers read without it).
pub fn read_file(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let _turn =
        (!BACKGROUND.with(|b| b.get())).then(|| READ.lock().unwrap_or_else(|e| e.into_inner()));
    let file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > limit {
        return Err(io::Error::other("file exceeds the read limit"));
    }
    let mut data = Vec::with_capacity(len as usize);
    file.take(limit).read_to_end(&mut data)?;
    Ok(data)
}

/// Full-size image sources: one file at a time, so a hard disk reads each
/// file in one sweep; the image on screen goes first. Background reads go in
/// chunks and step aside between them while the image on screen waits.
struct ImageTurn {
    busy: bool,
    urgent_waiting: usize,
}
struct ReadTurn {
    state: Mutex<ImageTurn>,
    wake: Condvar,
    /// Tests: whether each read that took the turn was urgent, in order.
    #[cfg(test)]
    taken: Mutex<Vec<bool>>,
}
static IMAGE_READS: ReadTurn = ReadTurn::new();
const IMAGE_CHUNK: u64 = 4 * 1024 * 1024;

/// Image read statistics for the diagnostic log (taken and reset every 5 s).
static READS_URGENT: AtomicUsize = AtomicUsize::new(0);
static READS_BACKGROUND: AtomicUsize = AtomicUsize::new(0);
static READS_STOPPED: AtomicUsize = AtomicUsize::new(0);
static STEPPED_ASIDE: AtomicUsize = AtomicUsize::new(0);
static READ_MS: AtomicUsize = AtomicUsize::new(0);
static WAIT_MS: AtomicUsize = AtomicUsize::new(0);
static READ_MIB: AtomicUsize = AtomicUsize::new(0);
static DEMOTED: AtomicUsize = AtomicUsize::new(0);
/// "urgent U background B (stopped S, stepped aside A, no longer on screen D) · R MiB in T ms, waited W ms".
pub fn take_read_stats() -> String {
    let take = |a: &AtomicUsize| a.swap(0, Ordering::Relaxed);
    format!(
        "urgent {} background {} (stopped {}, stepped aside {}, no longer on screen {}) · {} MiB in {} ms, waited {} ms",
        take(&READS_URGENT),
        take(&READS_BACKGROUND),
        take(&READS_STOPPED),
        take(&STEPPED_ASIDE),
        take(&DEMOTED),
        take(&READ_MIB),
        take(&READ_MS),
        take(&WAIT_MS)
    )
}

/// Reads a whole image file (at most `limit` bytes) in the image read turn.
/// `urgent`: the image is on screen (asked again each time the turn is
/// taken). `cancelled`: the image is no longer wanted; a waiting or reading
/// background read stops with `ErrorKind::Interrupted`.
pub fn read_image(
    path: &Path,
    limit: u64,
    urgent: impl Fn() -> bool,
    cancelled: impl Fn() -> bool,
) -> io::Result<Vec<u8>> {
    IMAGE_READS.read(path, limit, urgent, cancelled)
}
impl ReadTurn {
    const fn new() -> Self {
        Self {
            state: Mutex::new(ImageTurn {
                busy: false,
                urgent_waiting: 0,
            }),
            wake: Condvar::new(),
            #[cfg(test)]
            taken: Mutex::new(Vec::new()),
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, ImageTurn> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn read(
        &self,
        path: &Path,
        limit: u64,
        urgent: impl Fn() -> bool,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<Vec<u8>> {
        let mut file = fs::File::open(path)?;
        let len = file.metadata()?.len();
        if len > limit {
            return Err(io::Error::other("file exceeds the read limit"));
        }
        let stop = || {
            READS_STOPPED.fetch_add(1, Ordering::Relaxed);
            io::Error::new(io::ErrorKind::Interrupted, "cancelled")
        };
        let mut data = Vec::with_capacity(len as usize);
        let mut counted = false;
        let urgent_now = &urgent;
        loop {
            let mut urgent = urgent_now();
            if !counted {
                counted = true;
                if urgent {
                    &READS_URGENT
                } else {
                    &READS_BACKGROUND
                }
                .fetch_add(1, Ordering::Relaxed);
            }
            let asked = Instant::now();
            {
                let mut turn = self.lock();
                if urgent {
                    turn.urgent_waiting += 1;
                }
                while turn.busy || (!urgent && turn.urgent_waiting > 0) {
                    // Scrolled past while waiting: no longer ahead of the queue.
                    if urgent && !urgent_now() {
                        urgent = false;
                        turn.urgent_waiting -= 1;
                        DEMOTED.fetch_add(1, Ordering::Relaxed);
                    }
                    if !urgent && cancelled() {
                        return Err(stop());
                    }
                    turn = self
                        .wake
                        .wait_timeout(turn, Duration::from_millis(20))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                if urgent {
                    turn.urgent_waiting -= 1;
                }
                turn.busy = true;
                #[cfg(test)]
                self.taken
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(urgent);
            }
            WAIT_MS.fetch_add(asked.elapsed().as_millis() as usize, Ordering::Relaxed);
            let reading = Instant::now();
            // Read while holding the turn; a background read steps aside after a
            // chunk when the image on screen is waiting.
            // Urgency is asked again between chunks: an image scrolled past
            // while it was read is no longer on screen, so its read becomes a
            // background read (steps aside, stops when no longer wanted).
            let result = (|| -> io::Result<bool> {
                loop {
                    if !urgent && cancelled() {
                        return Err(stop());
                    }
                    let read = (&mut file).take(IMAGE_CHUNK).read_to_end(&mut data)?;
                    if read == 0 || data.len() as u64 >= len {
                        return Ok(true);
                    }
                    if urgent && !urgent_now() {
                        urgent = false;
                        DEMOTED.fetch_add(1, Ordering::Relaxed);
                    }
                    if !urgent && self.lock().urgent_waiting > 0 {
                        return Ok(false);
                    }
                }
            })();
            self.lock().busy = false;
            self.wake.notify_all();
            READ_MS.fetch_add(reading.elapsed().as_millis() as usize, Ordering::Relaxed);
            if matches!(result, Ok(false)) {
                STEPPED_ASIDE.fetch_add(1, Ordering::Relaxed);
            }
            match result {
                Ok(true) => {
                    READ_MIB.fetch_add(data.len() >> 20, Ordering::Relaxed);
                    // A file that grew while it was read stays within the limit.
                    if data.len() as u64 > limit {
                        return Err(io::Error::other("file exceeds the read limit"));
                    }
                    return Ok(data);
                }
                Ok(false) => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

pub fn set_grid_demand(tiles: usize) {
    GRID_DEMAND.store(tiles, Ordering::Relaxed);
}

pub fn grid_demand() -> usize {
    GRID_DEMAND.load(Ordering::Relaxed)
}

/// Background work waits (up to `max`) while visible grid tiles are loading.
pub fn yield_to_grid(max: Duration) {
    let started = Instant::now();
    while grid_demand() > 0 && started.elapsed() < max {
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_file_reads_respect_the_limit_and_yield_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        fs::write(&path, vec![7u8; 1000]).unwrap();
        assert_eq!(read_file(&path, 1000).unwrap().len(), 1000);
        assert!(read_file(&path, 999).is_err());
        // A background reader never waits for the turn a foreground reader holds.
        let held = READ.lock().unwrap();
        let background = path.clone();
        let read = std::thread::spawn(move || {
            set_background_reader();
            read_file(&background, 1000).unwrap().len()
        });
        assert_eq!(read.join().unwrap(), 1000);
        drop(held);
        set_grid_demand(3);
        let t = Instant::now();
        yield_to_grid(Duration::from_millis(30));
        assert!(t.elapsed() >= Duration::from_millis(30));
        set_grid_demand(0);
        let t = Instant::now();
        yield_to_grid(Duration::from_secs(5));
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn image_reads_take_turns_the_image_on_screen_first_and_stop_when_cancelled() {
        use std::sync::{atomic::AtomicBool, Arc};
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.bin");
        let small = dir.path().join("small.bin");
        let size = 3 * IMAGE_CHUNK as usize + 123;
        fs::write(&big, (0..size).map(|i| i as u8).collect::<Vec<_>>()).unwrap();
        fs::write(&small, vec![1u8; 10]).unwrap();
        // Whole and intact, within the limit only.
        let turn = Arc::new(ReadTurn::new());
        let data = turn.read(&big, u64::MAX, || false, || false).unwrap();
        assert_eq!(data.len(), size);
        assert!(data.iter().enumerate().all(|(i, b)| *b == i as u8));
        assert!(turn.read(&big, 1000, || true, || false).is_err());
        assert_eq!(
            read_image(&small, 100, || true, || false).unwrap().len(),
            10
        );
        // A background read waits while the turn is held and gives up when
        // its image is no longer wanted.
        let cancelled = Arc::new(AtomicBool::new(false));
        turn.lock().busy = true;
        let flag = cancelled.clone();
        let path = small.clone();
        let waiting_turn = turn.clone();
        let waiting = std::thread::spawn(move || {
            waiting_turn.read(&path, 100, || false, || flag.load(Ordering::Relaxed))
        });
        // Still waiting while the turn is held (it can only finish by being cancelled).
        std::thread::sleep(Duration::from_millis(60));
        assert!(!waiting.is_finished());
        cancelled.store(true, Ordering::Relaxed);
        let error = waiting.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        // Waiting background reads let an urgent one go first.
        let order = Arc::new(Mutex::new(Vec::new()));
        let spawn = |name: &'static str, urgent: bool| {
            let order = order.clone();
            let path = small.clone();
            let turn = turn.clone();
            std::thread::spawn(move || {
                turn.read(&path, 100, move || urgent, || false).unwrap();
                order.lock().unwrap().push(name);
            })
        };
        turn.taken.lock().unwrap().clear();
        let background = spawn("background", false);
        std::thread::sleep(Duration::from_millis(40));
        let urgent = spawn("urgent", true);
        // Until the urgent read is queued (not a fixed sleep: a loaded machine
        // may start the thread late).
        let deadline = Instant::now() + Duration::from_secs(10);
        while turn.lock().urgent_waiting == 0 {
            assert!(Instant::now() < deadline, "the urgent read never queued");
            std::thread::sleep(Duration::from_millis(2));
        }
        {
            let mut state = turn.lock();
            assert_eq!(state.urgent_waiting, 1);
            state.busy = false;
        }
        turn.wake.notify_all();
        urgent.join().unwrap();
        background.join().unwrap();
        // The urgent read took the turn first (completion order can differ).
        assert_eq!(*turn.taken.lock().unwrap(), vec![true, false]);
        assert_eq!(order.lock().unwrap().len(), 2);

        // An image scrolled past while its read waits: no longer ahead of the
        // queue, and it stops once it is not wanted.
        let turn = Arc::new(ReadTurn::new());
        turn.lock().busy = true;
        let on_screen = Arc::new(AtomicBool::new(true));
        let gone = Arc::new(AtomicBool::new(false));
        let (screen, wanted_gone, path, reader) =
            (on_screen.clone(), gone.clone(), small.clone(), turn.clone());
        let waiting = std::thread::spawn(move || {
            reader.read(
                &path,
                100,
                || screen.load(Ordering::Relaxed),
                || wanted_gone.load(Ordering::Relaxed),
            )
        });
        let until = |what: &str, done: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done() {
                assert!(Instant::now() < deadline, "{what}");
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        until("queued as urgent", &|| turn.lock().urgent_waiting == 1);
        on_screen.store(false, Ordering::Relaxed);
        until("demoted", &|| turn.lock().urgent_waiting == 0);
        gone.store(true, Ordering::Relaxed);
        assert_eq!(
            waiting.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }
}

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
        Mutex,
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
}

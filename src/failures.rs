//! Images NoirView could not read, for the notice and the list in Settings →
//! Performance: thumbnails, first views, keywords and full images, from the
//! viewer, the background pass and the folder watcher.
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Thumbnail,
    FirstView,
    Keywords,
    Image,
}
impl What {
    pub const ALL: [What; 4] = [
        What::Image,
        What::Thumbnail,
        What::FirstView,
        What::Keywords,
    ];
    pub fn label(self) -> &'static str {
        match self {
            What::Thumbnail => "thumbnails",
            What::FirstView => "first views",
            What::Keywords => "keywords",
            What::Image => "images",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}
#[derive(Clone, Debug)]
pub struct Failure {
    pub what: What,
    pub path: PathBuf,
    pub reason: String,
}
#[derive(Default)]
struct Log {
    recent: VecDeque<Failure>,
    counts: [u64; 4],
}
/// The newest failures kept for the list.
const RECENT: usize = 100;
static LOG: Mutex<Log> = Mutex::new(Log {
    recent: VecDeque::new(),
    counts: [0; 4],
});
/// Changes with every recorded failure (read without the lock).
static SERIAL: AtomicU64 = AtomicU64::new(0);

pub fn record(what: What, path: &Path, reason: impl std::fmt::Display) {
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.counts[what.index()] += 1;
    // The same image failing again replaces its older entry.
    log.recent.retain(|f| !(f.what == what && f.path == path));
    log.recent.push_front(Failure {
        what,
        path: path.to_path_buf(),
        reason: reason.to_string(),
    });
    log.recent.truncate(RECENT);
    SERIAL.fetch_add(1, Ordering::Relaxed);
}
pub fn serial() -> u64 {
    SERIAL.load(Ordering::Relaxed)
}
/// Counts by kind (in `What::ALL` order) and the newest failures first.
pub fn snapshot() -> ([u64; 4], Vec<Failure>) {
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let counts = What::ALL.map(|w| log.counts[w.index()]);
    (counts, log.recent.iter().cloned().collect())
}
pub fn clear() {
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    *log = Log::default();
    SERIAL.fetch_add(1, Ordering::Relaxed);
}
/// "2 thumbnails, 1 keywords" for the counts that are not zero.
pub fn describe(counts: [u64; 4]) -> String {
    What::ALL
        .iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .map(|(w, n)| format!("{n} {}", w.label()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_are_counted_listed_once_per_image_and_described() {
        // Other tests may record failures meanwhile: only these paths count.
        let (a, b) = (
            Path::new("failures-test/a.png"),
            Path::new("failures-test/b.jpg"),
        );
        let before = serial();
        let (old, _) = snapshot();
        record(What::Thumbnail, a, "bad header");
        record(What::Thumbnail, a, "still bad");
        record(What::Keywords, b, "unreadable XMP");
        assert!(serial() >= before + 3);
        let (counts, recent) = snapshot();
        assert!(counts[1] >= old[1] + 2 && counts[3] > old[3]);
        let ours: Vec<_> = recent
            .iter()
            .filter(|f| f.path.starts_with("failures-test"))
            .collect();
        assert_eq!(ours.len(), 2);
        assert_eq!(
            ours.iter().find(|f| f.path == a).unwrap().reason,
            "still bad"
        );
        assert_eq!(describe([0, 2, 0, 1]), "2 thumbnails, 1 keywords");
    }
}

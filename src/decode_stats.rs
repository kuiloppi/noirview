//! Where thumbnail/preview load time goes, for the diagnostic log. Counters
//! are process-wide atomics, summed between log lines (`take`).
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

#[derive(Clone, Copy)]
pub enum Phase {
    /// Whole worker job: cache lookup, read, decode, resize, convert.
    Job,
    /// Thumbnail served from the disk cache (part of Job).
    Cached,
    /// File read including the wait for the one-at-a-time disk turn.
    Read,
    /// Reduced-IDCT JPEG decode.
    Decode,
    /// Lanczos to the exact size and EXIF orientation.
    Resize,
    /// Storing the new thumbnail in the disk cache.
    Store,
    /// RGBA to egui texture data on the worker.
    Convert,
}
const PHASES: usize = 7;
const NAMES: [&str; PHASES] = [
    "job", "cached", "read", "decode", "resize", "store", "convert",
];
static COUNT: [AtomicU64; PHASES] = [const { AtomicU64::new(0) }; PHASES];
static MICROS: [AtomicU64; PHASES] = [const { AtomicU64::new(0) }; PHASES];
static IN_FLIGHT: AtomicU64 = AtomicU64::new(0);
static MAX_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);
/// Decoded sources: count, kilopixels, progressive count.
static SOURCES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

pub fn add(phase: Phase, started: Instant) {
    let i = phase as usize;
    COUNT[i].fetch_add(1, Ordering::Relaxed);
    MICROS[i].fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
}
pub fn source(dimensions: [u32; 2], progressive: bool) {
    SOURCES[0].fetch_add(1, Ordering::Relaxed);
    SOURCES[1].fetch_add(
        dimensions[0] as u64 * dimensions[1] as u64 / 1000,
        Ordering::Relaxed,
    );
    SOURCES[2].fetch_add(progressive as u64, Ordering::Relaxed);
}
/// A running worker job; dropping it records the job time.
pub struct Job(Instant);
impl Job {
    pub fn begin() -> Self {
        let now = IN_FLIGHT.fetch_add(1, Ordering::Relaxed) + 1;
        MAX_IN_FLIGHT.fetch_max(now, Ordering::Relaxed);
        Job(Instant::now())
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
        add(Phase::Job, self.0);
    }
}
/// "n×avg ms" per phase since the previous call; None when nothing ran.
pub fn take() -> Option<String> {
    let counts: Vec<u64> = COUNT.iter().map(|a| a.swap(0, Ordering::Relaxed)).collect();
    let micros: Vec<u64> = MICROS
        .iter()
        .map(|a| a.swap(0, Ordering::Relaxed))
        .collect();
    let sources: Vec<u64> = SOURCES
        .iter()
        .map(|a| a.swap(0, Ordering::Relaxed))
        .collect();
    let peak = MAX_IN_FLIGHT.swap(IN_FLIGHT.load(Ordering::Relaxed), Ordering::Relaxed);
    if counts.iter().all(|&n| n == 0) {
        return None;
    }
    let mut parts: Vec<String> = (0..PHASES)
        .filter(|&i| counts[i] > 0)
        .map(|i| {
            format!(
                "{} {}x{:.1}",
                NAMES[i],
                counts[i],
                micros[i] as f64 / counts[i] as f64 / 1000.
            )
        })
        .collect();
    parts.push(format!("parallel max {peak}"));
    if sources[0] > 0 {
        parts.push(format!(
            "src {:.1} Mpx avg, progressive {}/{}",
            sources[1] as f64 / sources[0] as f64 / 1000.,
            sources[2],
            sources[0]
        ));
    }
    Some(parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phases_sum_between_reads_and_reset() {
        // Other tests may record concurrently: only check presence and format.
        {
            let _job = Job::begin();
            add(Phase::Decode, Instant::now());
            source([2000, 1000], true);
        }
        let line = take().expect("recorded");
        assert!(line.contains("job ") && line.contains("decode ") && line.contains("parallel max"));
        assert!(line.contains("progressive"));
    }
}

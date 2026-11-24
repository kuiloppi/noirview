//! Bounded image work with foreground promotion and reusable in-flight decodes.
use crate::model::{Settings, Upscale};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub path: PathBuf,
    pub thumb: bool,
    pub preview: bool,
    pub revision: u64,
}
pub struct Request {
    pub key: Key,
    pub settings: Settings,
    pub max_side: u32,
    pub cancelled: AtomicBool,
    /// Scrolled past while running: its file read stops (freeing the disk for
    /// the image the view stops on); a decode past its read still finishes.
    pub unwanted: AtomicBool,
}
pub struct Prepared {
    /// When a worker began this image (page preparation time).
    pub started: std::time::Instant,
    pub color: egui::ColorImage,
    pub dimensions: [u32; 2],
    pub warning: Option<String>,
}
pub struct Loaded {
    pub request: Arc<Request>,
    pub prepared: Result<Prepared, String>,
}
impl Loaded {
    pub fn bytes(&self) -> usize {
        self.prepared
            .as_ref()
            .map(|p| p.color.pixels.len() * 4)
            .unwrap_or(0)
    }
}
#[derive(Debug)]
pub struct PrefetchDeferred;
impl std::fmt::Display for PrefetchDeferred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Prefetch is waiting for memory")
    }
}
impl std::error::Error for PrefetchDeferred {}

/// Maximum final RGBA size, before source dimensions are known. Decoder scratch
/// allocations remain subject to the separate decoder limit.
fn unknown_bytes(key: &Key, max_side: u32) -> usize {
    let side = if key.thumb {
        512
    } else if key.preview {
        1024
    } else {
        max_side.min(8192)
    } as usize;
    side * side * 4
}
#[derive(PartialEq, Eq)]
enum Stage {
    Queued,
    Running,
    Ready,
}
struct Job {
    request: Arc<Request>,
    stage: Stage,
    priority: bool,
}
#[derive(Default)]
struct State {
    /// The share of the memory budget usable now (`memory::Pressure`);
    /// None: all of it.
    share: Option<f64>,
    jobs: HashMap<Key, Job>,
    high: VecDeque<Key>,
    low: VecDeque<Key>,
    ready: VecDeque<Loaded>,
    ready_bytes: usize,
    ready_budget: usize,
    running: usize,
    foreground_busy: bool,
    focus: Vec<Key>,
    attempted_prefetch: HashSet<Key>,
    costs: HashMap<Key, usize>,
    resident_bytes: usize,
    output_budget: usize,
    stopped: bool,
    /// Running + ready results allowed at once; 0 means the default of 4.
    slots: usize,
    /// Cancel running work that is no longer wanted (grid thumbnails: a page
    /// left behind should not delay the page now on screen).
    cancel_running: bool,
}
#[derive(Default)]
pub struct WorkQueue {
    state: Mutex<State>,
    wake: Condvar,
}
impl State {
    fn slots(&self) -> usize {
        if self.slots == 0 {
            4
        } else {
            self.slots
        }
    }
    fn reserved_bytes(&self) -> usize {
        self.resident_bytes
            .saturating_add(self.ready_bytes)
            .saturating_add(
                self.jobs
                    .iter()
                    .filter(|(_, j)| {
                        j.stage == Stage::Running || (j.priority && j.stage == Stage::Queued)
                    })
                    .map(|(key, j)| {
                        self.costs
                            .get(key)
                            .copied()
                            .unwrap_or_else(|| unknown_bytes(key, j.request.max_side))
                    })
                    .sum::<usize>(),
            )
    }
    fn discard_ready(&mut self, index: usize) {
        if let Some(loaded) = self.ready.remove(index) {
            self.ready_bytes -= loaded.bytes();
            self.jobs.remove(&loaded.request.key);
        }
    }
    fn priority(&self, key: &Key) -> bool {
        self.jobs.get(key).is_some_and(|j| j.priority)
    }
}
impl WorkQueue {
    /// A queue that lets `slots` results be running or waiting for upload at
    /// once (the default queue allows 4). Used for small grid thumbnails.
    pub fn with_slots(slots: usize) -> Self {
        let queue = Self::default();
        let mut s = queue.state.lock().unwrap();
        s.slots = slots.max(4);
        s.cancel_running = true;
        drop(s);
        queue
    }
    /// A queue for full-size images with `slots` results running or waiting
    /// for upload at once. Unlike thumbnails, running work is never cancelled
    /// just because navigation moved on.
    pub fn sized(slots: usize) -> Self {
        let queue = Self::default();
        queue.state.lock().unwrap().slots = slots.max(4);
        queue
    }
    /// Snapshot only this bounded horizon and active work. Missing dimensions are
    /// probed by a worker, never by the UI thread.
    pub fn costs(&self) -> HashMap<Key, usize> {
        self.state.lock().unwrap().costs.clone()
    }
    pub fn resident(&self, bytes: usize) {
        self.state.lock().unwrap().resident_bytes = bytes;
    }
    pub fn retain_costs(&self, horizon: &HashSet<Key>) {
        let mut s = self.state.lock().unwrap();
        let retained: HashSet<_> = horizon.iter().chain(s.jobs.keys()).cloned().collect();
        s.costs.retain(|key, _| retained.contains(key));
        s.attempted_prefetch.retain(|key| retained.contains(key));
    }
    pub fn orphan_bytes(&self, wanted: &HashSet<Key>) -> usize {
        let s = self.state.lock().unwrap();
        s.jobs
            .iter()
            .filter(|(key, j)| !wanted.contains(*key) && j.stage == Stage::Running)
            .map(|(key, j)| {
                s.costs
                    .get(key)
                    .copied()
                    .unwrap_or_else(|| unknown_bytes(key, j.request.max_side))
            })
            .sum::<usize>()
            + s.ready
                .iter()
                .filter(|r| !wanted.contains(&r.request.key))
                .map(Loaded::bytes)
                .sum::<usize>()
    }
    /// Called after opening the decoder header and before allocating its output.
    /// Promotion to foreground is checked under the same lock as admission.
    pub fn reserve_output(&self, request: &Request, bytes: usize) -> bool {
        let mut s = self.state.lock().unwrap();
        if !s
            .jobs
            .get(&request.key)
            .is_some_and(|j| std::ptr::eq(j.request.as_ref(), request))
        {
            return false;
        }
        s.costs.insert(request.key.clone(), bytes);
        self.wake.notify_all();
        s.priority(&request.key) || s.reserved_bytes() <= s.output_budget
    }
    pub fn defer(&self, request: Arc<Request>, background: bool) {
        let mut s = self.state.lock().unwrap();
        s.running -= 1;
        if !background {
            s.foreground_busy = false;
        }
        if let Some(job) = s
            .jobs
            .get_mut(&request.key)
            .filter(|j| Arc::ptr_eq(&j.request, &request))
        {
            job.stage = Stage::Queued;
            if job.priority {
                s.high.push_back(request.key.clone());
            } else {
                s.low.push_back(request.key.clone());
            }
        }
        self.wake.notify_all();
    }
    /// Is this request's image needed on screen now?
    pub fn is_priority(&self, request: &Request) -> bool {
        let s = self.state.lock().unwrap();
        s.jobs
            .get(&request.key)
            .is_some_and(|j| j.priority && std::ptr::eq(j.request.as_ref(), request))
    }
    /// Use only this share of the memory budget (memory pressure).
    pub fn set_share(&self, share: f64) {
        self.state.lock().unwrap().share = Some(share);
    }
    pub fn plan(
        &self,
        focus: &[Key],
        high: &[Key],
        low: &[Key],
        settings: &Settings,
        max_side: u32,
    ) {
        let mut s = self.state.lock().unwrap();
        if s.stopped {
            return;
        }
        if s.focus != focus {
            s.focus = focus.to_vec();
            s.attempted_prefetch.clear();
        }
        let budget =
            (settings.cache_mb.max(64) as f64 * 1024. * 1024. * s.share.unwrap_or(1.)) as usize;
        s.ready_budget = (budget / 4).clamp(16 * 1024 * 1024, 256 * 1024 * 1024);
        s.output_budget = budget;
        // UI pages are capped at 64; also bound admission at this boundary.
        let high = &high[..high.len().min(64)];
        let low = &low[..low.len().min(64)];
        let wanted: HashSet<_> = high.iter().chain(low).collect();
        let cancel_running = s.cancel_running;
        s.jobs.retain(|key, job| {
            job.priority = high.contains(key);
            if wanted.contains(key) || focus.contains(key) {
                job.request.unwanted.store(false, Ordering::Relaxed);
                return true;
            }
            if job.stage == Stage::Queued
                || (job.stage == Stage::Running && cancel_running)
                || (job.stage == Stage::Running
                    && (key.preview
                        || (job.request.settings.upscale == Upscale::Onnx && !key.thumb)))
            {
                job.request.cancelled.store(true, Ordering::Relaxed);
                return false;
            }
            // Ordinary in-flight decoding remains useful cache work. Navigation does
            // not invalidate the source or processing parameters; only the
            // file read stops.
            job.request.unwanted.store(true, Ordering::Relaxed);
            true
        });
        s.high.clear();
        s.low.clear();
        for (keys, priority) in [(high, true), (low, false)] {
            for key in keys {
                if !s.jobs.contains_key(key) {
                    if (!priority || key.preview) && s.attempted_prefetch.contains(key) {
                        continue;
                    }
                    let request = Arc::new(Request {
                        key: key.clone(),
                        settings: settings.clone(),
                        max_side,
                        cancelled: AtomicBool::new(false),
                        unwanted: AtomicBool::new(false),
                    });
                    s.jobs.insert(
                        key.clone(),
                        Job {
                            request,
                            stage: Stage::Queued,
                            priority,
                        },
                    );
                }
                let job = s.jobs.get(key).unwrap();
                if job.stage == Stage::Queued {
                    if priority {
                        if !s.high.contains(key) {
                            s.high.push_back(key.clone());
                        }
                    } else if !high.contains(key) && !s.low.contains(key) {
                        s.low.push_back(key.clone());
                    }
                }
            }
        }
        self.wake.notify_all();
    }
    /// One worker calls this with false and never takes speculative work.
    pub fn take(&self, background: bool) -> Option<Arc<Request>> {
        let mut s = self.state.lock().unwrap();
        loop {
            if s.stopped {
                return None;
            }
            let high = !s.high.is_empty();
            let low_index = if background && !high {
                let used = s.reserved_bytes();
                s.low.iter().position(|key| {
                    s.costs
                        .get(key)
                        .is_none_or(|bytes| used.saturating_add(*bytes) <= s.output_budget)
                })
            } else {
                None
            };
            let available = high || low_index.is_some();
            // Let the reserved foreground worker take the first visible request.
            let leave_for_foreground = background && high && !s.foreground_busy;
            if available && !leave_for_foreground {
                // Reserve a result slot for foreground work.
                let limit = if high { s.slots() } else { s.slots() - 1 };
                while high
                    && (s.ready.len() + s.running >= limit || s.ready_bytes >= s.ready_budget)
                {
                    if let Some(i) = s
                        .ready
                        .iter()
                        .position(|r| !s.priority(&r.request.key) || r.request.key.preview)
                    {
                        s.discard_ready(i);
                    } else {
                        break;
                    }
                }
                if s.ready.len() + s.running < limit
                    && (s.ready_bytes < s.ready_budget || s.ready.is_empty())
                {
                    let key = if high {
                        s.high.pop_front()
                    } else {
                        s.low.remove(low_index.unwrap())
                    }
                    .unwrap();
                    let job = s.jobs.get_mut(&key).unwrap();
                    job.stage = Stage::Running;
                    let request = job.request.clone();
                    s.running += 1;
                    if !background {
                        s.foreground_busy = true;
                    }
                    self.wake.notify_all();
                    return Some(request);
                }
            }
            s = self.wake.wait(s).unwrap();
        }
    }
    pub fn complete(
        &self,
        request: Arc<Request>,
        prepared: Result<Prepared, String>,
        background: bool,
    ) {
        let mut s = self.state.lock().unwrap();
        s.running -= 1;
        if !background {
            s.foreground_busy = false;
        }
        // A read stopped because the image was scrolled past: forgotten, so a
        // later visit starts it again (not a failure).
        if prepared.is_err()
            && request.unwanted.load(Ordering::Relaxed)
            && s.jobs
                .get(&request.key)
                .is_some_and(|j| Arc::ptr_eq(&j.request, &request))
        {
            s.jobs.remove(&request.key);
            self.wake.notify_all();
            return;
        }
        if !request.cancelled.load(Ordering::Relaxed)
            && s.jobs
                .get(&request.key)
                .is_some_and(|j| Arc::ptr_eq(&j.request, &request))
        {
            let job = s.jobs.get_mut(&request.key).unwrap();
            job.stage = Stage::Ready;
            if !job.priority || job.request.key.preview {
                s.attempted_prefetch.insert(request.key.clone());
            }
            let loaded = Loaded { request, prepared };
            if loaded.prepared.is_ok() {
                s.costs.insert(loaded.request.key.clone(), loaded.bytes());
            }
            s.ready_bytes += loaded.bytes();
            s.ready.push_back(loaded);
            // Preserve at least one oversized visible result so it can make progress.
            while s.ready.len() > 1 && (s.ready.len() > s.slots() || s.ready_bytes > s.ready_budget)
            {
                let index = s
                    .ready
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, r)| {
                        if !s.priority(&r.request.key) {
                            0
                        } else if r.request.key.preview {
                            1
                        } else {
                            2
                        }
                    })
                    .map(|(i, _)| i)
                    .unwrap();
                s.discard_ready(index);
            }
        }
        self.wake.notify_all();
    }
    pub fn take_ready(&self, max_bytes: Option<usize>) -> Option<Loaded> {
        let mut s = self.state.lock().unwrap();
        let priority_index = s
            .ready
            .iter()
            .position(|r| s.priority(&r.request.key) && !r.request.key.preview)
            .or_else(|| s.ready.iter().position(|r| s.priority(&r.request.key)));
        if priority_index.is_none()
            && s.jobs
                .values()
                .any(|j| j.priority && j.stage != Stage::Ready)
        {
            return None;
        }
        let index = priority_index.or_else(|| (!s.ready.is_empty()).then_some(0))?;
        if max_bytes.is_some_and(|max| s.ready[index].bytes() > max) {
            return None;
        }
        let loaded = s.ready.remove(index).unwrap();
        s.ready_bytes -= loaded.bytes();
        // Keep the output charged while the UI transfers it into a texture.
        // The next plan replaces this with the actual retained texture total.
        s.resident_bytes = s.resident_bytes.saturating_add(loaded.bytes());
        s.jobs.remove(&loaded.request.key);
        self.wake.notify_all();
        Some(loaded)
    }
    /// Queued or running work exists (used to keep background passes idle).
    pub fn busy(&self) -> bool {
        let s = self.state.lock().unwrap();
        !s.high.is_empty() || !s.low.is_empty() || s.running > 0
    }
    pub fn has_ready(&self) -> bool {
        let s = self.state.lock().unwrap();
        !s.ready.is_empty()
            && (s.ready.iter().any(|r| s.priority(&r.request.key))
                || !s
                    .jobs
                    .values()
                    .any(|j| j.priority && j.stage != Stage::Ready))
    }
    pub fn invalidate_paths(&self, paths: &HashSet<PathBuf>) {
        let mut s = self.state.lock().unwrap();
        s.jobs.retain(|key, job| {
            if paths.contains(&key.path) {
                job.request.cancelled.store(true, Ordering::Relaxed);
                false
            } else {
                true
            }
        });
        s.high.retain(|k| !paths.contains(&k.path));
        s.low.retain(|k| !paths.contains(&k.path));
        s.ready.retain(|r| !paths.contains(&r.request.key.path));
        s.ready_bytes = s.ready.iter().map(Loaded::bytes).sum();
        s.costs.retain(|k, _| !paths.contains(&k.path));
        s.attempted_prefetch.retain(|k| !paths.contains(&k.path));
        self.wake.notify_all();
    }
    pub fn clear(&self) {
        let mut s = self.state.lock().unwrap();
        for job in s.jobs.values() {
            job.request.cancelled.store(true, Ordering::Relaxed);
        }
        s.jobs.clear();
        s.high.clear();
        s.low.clear();
        s.ready.clear();
        s.ready_bytes = 0;
        s.focus.clear();
        s.attempted_prefetch.clear();
        s.costs.clear();
        s.resident_bytes = 0;
        self.wake.notify_all();
    }
    pub fn shutdown(&self) {
        self.clear();
        self.state.lock().unwrap().stopped = true;
        self.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(name: &str) -> Key {
        Key {
            path: name.into(),
            thumb: false,
            preview: false,
            revision: 0,
        }
    }
    #[test]
    fn a_read_scrolled_past_stops_without_a_failure_and_starts_again_on_return() {
        let q = WorkQueue::default();
        let settings = Settings::default();
        let (a, b) = (key("a"), key("b"));
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[],
            &settings,
            8192,
        );
        let running = q.take(false).unwrap();
        assert!(q.is_priority(&running));
        // Scrolled on: the read of a is to stop, but the job is kept.
        q.plan(
            std::slice::from_ref(&b),
            std::slice::from_ref(&b),
            &[],
            &settings,
            8192,
        );
        assert!(running.unwanted.load(Ordering::Relaxed));
        assert!(!running.cancelled.load(Ordering::Relaxed));
        assert!(!q.is_priority(&running));
        // Its read stopped: forgotten, not a failure.
        q.complete(running, Err("cancelled".into()), false);
        assert!(!q.state.lock().unwrap().jobs.contains_key(&a));
        // Back to a: a new job starts it again.
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[],
            &settings,
            8192,
        );
        let again = q.take(false).unwrap();
        assert_eq!(again.key, a);
        assert!(!again.unwanted.load(Ordering::Relaxed));
        // Wanted again while still running: the read goes on.
        q.plan(
            std::slice::from_ref(&b),
            std::slice::from_ref(&b),
            &[],
            &settings,
            8192,
        );
        assert!(again.unwanted.load(Ordering::Relaxed));
        q.plan(
            std::slice::from_ref(&b),
            std::slice::from_ref(&b),
            std::slice::from_ref(&a),
            &settings,
            8192,
        );
        assert!(!again.unwanted.load(Ordering::Relaxed));
    }
    #[test]
    fn thumbnail_queue_cancels_running_work_for_a_page_left_behind() {
        let settings = Settings::default();
        for (queue, cancels) in [
            (WorkQueue::with_slots(12), true),
            (WorkQueue::default(), false),
        ] {
            let (a, b) = (key("old-page"), key("new-page"));
            queue.plan(
                std::slice::from_ref(&a),
                std::slice::from_ref(&a),
                &[],
                &settings,
                8192,
            );
            let running = queue.take(false).unwrap();
            assert_eq!(running.key, a);
            queue.plan(
                std::slice::from_ref(&b),
                std::slice::from_ref(&b),
                &[],
                &settings,
                8192,
            );
            assert_eq!(running.cancelled.load(Ordering::Relaxed), cancels);
            assert!(queue.busy());
            queue.complete(running, prepared(4), false);
            let next = queue.take(false).unwrap();
            assert_eq!(next.key, b);
            queue.complete(next, prepared(4), false);
            assert!(!queue.busy());
        }
    }
    fn prepared(bytes: usize) -> Result<Prepared, String> {
        Ok(Prepared {
            started: std::time::Instant::now(),
            color: egui::ColorImage::new([bytes / 4, 1], egui::Color32::WHITE),
            dimensions: [bytes as u32 / 4, 1],
            warning: None,
        })
    }
    #[test]
    fn optional_preview_never_evicts_or_delays_ready_full_detail_under_memory_pressure() {
        for full_bytes in [8, 20 * 1024 * 1024] {
            for preview_first in [true, false] {
                let q = WorkQueue::default();
                let full = key("photo.jpg");
                let preview = Key {
                    preview: true,
                    ..full.clone()
                };
                let focus = [full.clone(), preview.clone()];
                let settings = Settings {
                    cache_mb: 64,
                    ..Default::default()
                };
                q.plan(&focus, &focus, &[], &settings, 8192);
                let full_request = q.take(false).unwrap();
                let preview_request = q.take(true).unwrap();
                if preview_first {
                    q.complete(preview_request, prepared(4), true);
                    q.complete(full_request, prepared(full_bytes), false);
                } else {
                    q.complete(full_request, prepared(full_bytes), false);
                    q.complete(preview_request, prepared(4), true);
                }
                assert_eq!(q.take_ready(None).unwrap().request.key, full);
                while q.take_ready(None).is_some() {}
                q.plan(&focus, std::slice::from_ref(&preview), &[], &settings, 8192);
                assert!(q.state.lock().unwrap().high.is_empty());
            }
        }
    }
    #[test]
    fn deeper_horizon_is_bounded_and_zero_prunes_queued_speculation() {
        let q = WorkQueue::default();
        let a = key("current");
        let low: Vec<_> = (0..100).map(|i| key(&i.to_string())).collect();
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &low,
            &Settings::default(),
            8192,
        );
        assert_eq!(q.state.lock().unwrap().low.len(), 64);
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[],
            &Settings::default(),
            8192,
        );
        let s = q.state.lock().unwrap();
        assert!(s.low.is_empty());
        assert_eq!(s.jobs.len(), 1);
    }
    #[test]
    fn header_admission_counts_running_outputs_and_promotion_unblocks_large_images() {
        let mib = 1024 * 1024;
        let q = WorkQueue::default();
        let a = key("a");
        let b = key("b");
        let settings = Settings {
            cache_mb: 64,
            ..Default::default()
        };
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            std::slice::from_ref(&b),
            &settings,
            8192,
        );
        let foreground = q.take(false).unwrap();
        assert!(q.reserve_output(&foreground, 40 * mib));
        let prefetch = q.take(true).unwrap();
        assert!(!q.reserve_output(&prefetch, 32 * mib));
        q.defer(prefetch.clone(), true);
        assert_eq!(q.state.lock().unwrap().running, 1);
        assert_eq!(q.costs()[&b], 32 * mib);
        q.plan(
            std::slice::from_ref(&b),
            std::slice::from_ref(&b),
            &[],
            &settings,
            8192,
        );
        let promoted = q.take(true).unwrap();
        assert!(Arc::ptr_eq(&promoted, &prefetch));
        assert!(q.reserve_output(&promoted, 80 * mib)); // foreground can progress even above the target
        q.complete(promoted, prepared(4), true);
        q.complete(foreground, prepared(4), false);
        assert_eq!(q.take_ready(None).unwrap().request.key, b);
    }
    #[test]
    fn output_transfer_remains_charged_and_header_costs_survive_until_scope_changes() {
        let mib = 1024 * 1024;
        let q = WorkQueue::default();
        let a = key("a");
        let b = key("b");
        let settings = Settings {
            cache_mb: 64,
            ..Default::default()
        };
        q.plan(&[], &[], &[a.clone(), b.clone()], &settings, 8192);
        let first = q.take(true).unwrap();
        assert!(q.reserve_output(&first, 32 * mib));
        q.complete(first, prepared(32 * mib), true);
        let loaded = q.take_ready(None).unwrap();
        assert_eq!(loaded.bytes(), 32 * mib);
        let second = q.take(true).unwrap();
        assert!(!q.reserve_output(&second, 40 * mib));
        q.defer(second, true);
        q.plan(&[], &[], &[], &settings, 8192);
        q.retain_costs(&HashSet::from([b.clone()]));
        assert_eq!(q.costs()[&b], 40 * mib);
        q.retain_costs(&HashSet::new());
        assert!(q.costs().is_empty());
    }
    #[test]
    fn in_flight_prefetch_is_promoted_and_delivered_without_restart() {
        let q = WorkQueue::default();
        let a = key("a");
        let b = key("b");
        let c = key("c");
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[b.clone(), c.clone()],
            &Settings::default(),
            8192,
        );
        let running_a = q.take(false).unwrap();
        let running_b = q.take(true).unwrap();
        q.plan(
            std::slice::from_ref(&b),
            std::slice::from_ref(&b),
            std::slice::from_ref(&c),
            &Settings::default(),
            8192,
        );
        assert!(Arc::ptr_eq(
            &q.state.lock().unwrap().jobs[&b].request,
            &running_b
        ));
        assert!(!running_b.cancelled.load(Ordering::Relaxed));
        q.complete(running_a, prepared(4), false);
        assert!(q.take_ready(None).is_none()); // old image cannot delay the foreground
        q.complete(running_b, prepared(4), true);
        assert_eq!(q.take_ready(None).unwrap().request.key, b);
        assert_eq!(q.take_ready(None).unwrap().request.key, a);
    }
    #[test]
    fn queued_prefetch_moves_ahead_of_other_speculative_work() {
        let q = WorkQueue::default();
        let a = key("a");
        let b = key("b");
        let c = key("c");
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[b.clone(), c.clone()],
            &Settings::default(),
            8192,
        );
        let first = q.take(false).unwrap();
        let original = q.state.lock().unwrap().jobs[&c].request.clone();
        q.plan(
            std::slice::from_ref(&c),
            std::slice::from_ref(&c),
            std::slice::from_ref(&b),
            &Settings::default(),
            8192,
        );
        let promoted = q.take(true).unwrap();
        assert!(Arc::ptr_eq(&promoted, &original));
        assert_eq!(promoted.key, c);
        q.complete(first, prepared(4), false);
        q.complete(promoted, prepared(4), true);
    }
    #[test]
    fn path_invalidation_keeps_other_ready_pixels_and_rejects_late_old_completion() {
        let q = WorkQueue::default();
        let a = key("edited");
        let b = key("unchanged");
        let focus = [a.clone(), b.clone()];
        q.plan(&focus, &focus, &[], &Settings::default(), 8192);
        let old = q.take(false).unwrap();
        let other = q.take(true).unwrap();
        assert_eq!(old.key, a);
        assert_eq!(other.key, b);
        q.complete(other, prepared(4), true);
        q.invalidate_paths(&HashSet::from([a.path.clone()]));
        assert!(old.cancelled.load(Ordering::Relaxed));
        assert_eq!(q.take_ready(None).unwrap().request.key, b);
        q.plan(
            &focus,
            std::slice::from_ref(&a),
            &[],
            &Settings::default(),
            8192,
        );
        let new = q.take(true).unwrap();
        q.complete(old, prepared(4), false);
        assert!(q.take_ready(None).is_none());
        assert!(q.state.lock().unwrap().jobs.contains_key(&a));
        q.complete(new, prepared(4), true);
        assert_eq!(q.take_ready(None).unwrap().request.key, a);
    }
    #[test]
    fn invalidate_cannot_deliver_old_pixels_or_remove_a_replacement_job() {
        let q = WorkQueue::default();
        let a = key("a");
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[],
            &Settings::default(),
            8192,
        );
        let old = q.take(false).unwrap();
        q.clear();
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[],
            &Settings::default(),
            8192,
        );
        q.complete(old, prepared(4), false);
        assert!(q.take_ready(None).is_none());
        let new = q.take(false).unwrap();
        q.complete(new, prepared(8), false);
        assert_eq!(q.take_ready(None).unwrap().bytes(), 8);
    }
    #[test]
    fn obsolete_preview_is_cancelled_and_cannot_erase_a_later_return_to_the_same_key() {
        let q = WorkQueue::default();
        let full = key("photo");
        let preview = Key {
            preview: true,
            ..full.clone()
        };
        let demand = [full.clone(), preview.clone()];
        let settings = Settings::default();
        q.plan(&demand, &demand, &[], &settings, 8192);
        let ongoing_full = q.take(false).unwrap();
        let old = q.take(true).unwrap();
        assert!(old.key.preview);
        q.plan(std::slice::from_ref(&full), &[], &[], &settings, 8192);
        assert!(old.cancelled.load(Ordering::Relaxed));
        assert!(!ongoing_full.cancelled.load(Ordering::Relaxed));
        q.plan(&demand, &demand, &[], &settings, 8192);
        q.complete(old, Err("cancelled".into()), true);
        let new = q.take(true).unwrap();
        assert_eq!(new.key, preview);
        assert!(!new.cancelled.load(Ordering::Relaxed));
        q.complete(new, prepared(4), true);
        q.complete(ongoing_full, prepared(4), false);
        assert_eq!(q.take_ready(None).unwrap().request.key, full);
    }
    #[test]
    fn obsolete_ai_is_cancelled_but_a_return_to_the_same_key_can_restart() {
        let q = WorkQueue::default();
        let a = key("a");
        let b = key("b");
        let s = Settings {
            upscale: Upscale::Onnx,
            ..Default::default()
        };
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[],
            &s,
            8192,
        );
        let old = q.take(false).unwrap();
        q.plan(
            std::slice::from_ref(&b),
            std::slice::from_ref(&b),
            &[],
            &s,
            8192,
        );
        assert!(old.cancelled.load(Ordering::Relaxed));
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &[],
            &s,
            8192,
        );
        q.complete(old, Err("cancelled".into()), false);
        let new = q.take(false).unwrap();
        assert_eq!(new.key, a);
        assert!(!new.cancelled.load(Ordering::Relaxed));
        q.complete(new, prepared(4), false);
        assert!(q.take_ready(None).unwrap().prepared.is_ok());
    }
    #[test]
    fn completed_buffer_budget_keeps_foreground_and_limits_speculation() {
        let q = WorkQueue::default();
        let a = key("a");
        let b = key("b");
        let s = Settings {
            cache_mb: 64,
            ..Default::default()
        };
        q.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            std::slice::from_ref(&b),
            &s,
            8192,
        );
        let current = q.take(false).unwrap();
        let background = q.take(true).unwrap();
        q.complete(background, prepared(16 * 1024 * 1024), true);
        q.complete(current, prepared(20 * 1024 * 1024), false);
        {
            let state = q.state.lock().unwrap();
            assert_eq!(state.ready.len(), 1);
            assert_eq!(state.ready_bytes, 20 * 1024 * 1024);
        }
        assert!(q.take_ready(Some(16 * 1024 * 1024)).is_none());
        assert_eq!(q.take_ready(None).unwrap().request.key, a); // one oversized current image can progress
    }
    #[test]
    fn unchanged_focus_does_not_continuously_redecode_evicted_prefetch() {
        let q = WorkQueue::default();
        let a = key("a");
        let b = key("b");
        let c = key("c");
        q.plan(
            std::slice::from_ref(&a),
            &[],
            &[b.clone(), b.clone()],
            &Settings::default(),
            8192,
        );
        assert_eq!(q.state.lock().unwrap().low.len(), 1);
        let request = q.take(true).unwrap();
        q.complete(request, prepared(4), true);
        q.take_ready(None).unwrap();
        q.plan(
            std::slice::from_ref(&a),
            &[],
            std::slice::from_ref(&b),
            &Settings::default(),
            8192,
        );
        assert!(q.state.lock().unwrap().low.is_empty());
        q.plan(
            std::slice::from_ref(&c),
            &[],
            std::slice::from_ref(&b),
            &Settings::default(),
            8192,
        );
        assert_eq!(q.state.lock().unwrap().low.len(), 1);
    }
    #[test]
    fn rapid_navigation_prunes_queued_jobs_instead_of_growing_a_backlog() {
        let q = WorkQueue::default();
        for i in 0..5000 {
            let current = key(&i.to_string());
            let next = key(&(i + 1).to_string());
            q.plan(
                std::slice::from_ref(&current),
                std::slice::from_ref(&current),
                std::slice::from_ref(&next),
                &Settings::default(),
                8192,
            );
            let state = q.state.lock().unwrap();
            assert_eq!(state.jobs.len(), 2);
            assert_eq!(state.high.len(), 1);
            assert_eq!(state.low.len(), 1);
        }
    }
}

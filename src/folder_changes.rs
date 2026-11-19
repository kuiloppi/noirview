//! Coalesced filesystem notifications, independent of indexing and UI backpressure.
use super::{relevant, Revisions, Update};
use crate::model;
use crossbeam_channel::{bounded, Receiver, Sender};
use notify::{event::ModifyKind, EventKind};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
const MAX_PATHS: usize = 4096;
const SETTLE: Duration = Duration::from_millis(120);
#[derive(Default)]
pub(super) struct Changes {
    pub dirty: HashSet<PathBuf>,
    pub rescan: bool,
    pub refresh_all: bool,
    pub warning: Option<String>,
    since: Option<Instant>,
}
impl Changes {
    fn any(&self) -> bool {
        self.rescan || self.refresh_all || !self.dirty.is_empty() || self.warning.is_some()
    }
}
#[derive(Default)]
struct Pixels {
    paths: HashSet<PathBuf>,
    all: bool,
    since: Option<Instant>,
}
pub(super) struct ChangeInbox {
    pending: Mutex<Changes>,
    pixels: Mutex<Pixels>,
    pub signal: Receiver<()>,
    tx: Sender<()>,
    stopped: AtomicBool,
    revisions: Revisions,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl ChangeInbox {
    pub fn new(revisions: Revisions, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, signal) = bounded(1);
        Self {
            pending: Mutex::default(),
            pixels: Mutex::default(),
            signal,
            tx,
            stopped: AtomicBool::new(false),
            revisions,
            wake,
        }
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.tx.try_send(());
    }
    pub fn record(&self, event: notify::Result<notify::Event>) {
        if self.stopped.load(Ordering::Relaxed) || event.as_ref().is_ok_and(|e| !relevant(e)) {
            return;
        }
        let mut pending = self.pending.lock().unwrap();
        let mut pixels = self.pixels.lock().unwrap();
        let was_empty = !pixels.all && pixels.paths.is_empty();
        match event {
            Err(error) => {
                pending.warning = Some(format!("Folder watcher: {error}"));
                pending.rescan = true;
                pending.refresh_all = true;
            }
            Ok(event) => {
                if event.need_rescan()
                    || event.paths.is_empty()
                    || matches!(event.kind, EventKind::Any)
                {
                    pending.rescan = true;
                    pending.refresh_all = true;
                }
                if !matches!(
                    event.kind,
                    EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_))
                ) {
                    pending.rescan = true;
                }
                for path in event.paths {
                    if pending.refresh_all {
                        break;
                    }
                    let image = if model::supported(&path) {
                        if !pixels.all {
                            pixels.paths.insert(path.clone());
                        }
                        Some(path)
                    } else if path
                        .extension()
                        .is_some_and(|s| s.eq_ignore_ascii_case("xmp"))
                    {
                        let image = path.with_extension("");
                        model::supported(&image).then_some(image)
                    } else {
                        pending.rescan = true;
                        None
                    };
                    if let Some(path) = image {
                        self.revisions.external(&path);
                        if !pending.refresh_all {
                            pending.dirty.insert(path);
                        }
                    }
                    if pending.dirty.len() > MAX_PATHS || pixels.paths.len() > MAX_PATHS {
                        pending.rescan = true;
                        pending.refresh_all = true;
                    }
                    if pending.refresh_all {
                        pending.dirty.clear();
                        pixels.paths.clear();
                        pixels.all = true;
                    }
                }
            }
        }
        if pending.refresh_all {
            pending.dirty.clear();
            pixels.paths.clear();
            pixels.all = true;
            self.revisions.reconcile();
        } else if pending.rescan {
            self.revisions.0.lock().unwrap().serial += 1;
        }
        if pending.any() {
            pending.since.get_or_insert(Instant::now());
        }
        if was_empty && (pixels.all || !pixels.paths.is_empty()) {
            pixels.since = Some(Instant::now());
        }
        drop(pixels);
        drop(pending);
        let _ = self.tx.try_send(());
        (self.wake)();
    }
    pub fn internal(&self, path: PathBuf) {
        let mut pending = self.pending.lock().unwrap();
        if !pending.refresh_all {
            pending.dirty.insert(path);
            if pending.dirty.len() > MAX_PATHS {
                pending.dirty.clear();
                pending.refresh_all = true;
                pending.rescan = true;
                self.revisions.reconcile();
                let mut pixels = self.pixels.lock().unwrap();
                pixels.paths.clear();
                pixels.all = true;
                pixels.since.get_or_insert(Instant::now());
            }
        }
        pending.since.get_or_insert(Instant::now());
        drop(pending);
        let _ = self.tx.try_send(());
    }
    pub fn take(&self) -> Changes {
        let mut pending = self.pending.lock().unwrap();
        // Consume the coalesced wake while holding the same lock as record().
        while self.signal.try_recv().is_ok() {}
        std::mem::take(&mut *pending)
    }
    pub fn delay(&self) -> Option<Duration> {
        let pending = self.pending.lock().unwrap();
        pending
            .any()
            .then(|| SETTLE.saturating_sub(pending.since.map(|t| t.elapsed()).unwrap_or_default()))
    }
    pub fn pixel_delay(&self) -> Option<Duration> {
        let pixels = self.pixels.lock().unwrap();
        (pixels.all || !pixels.paths.is_empty())
            .then(|| SETTLE.saturating_sub(pixels.since.map(|t| t.elapsed()).unwrap_or_default()))
    }
    pub fn take_pixels(&self) -> Option<Update> {
        let mut pixels = self.pixels.lock().unwrap();
        if pixels.since.is_some_and(|t| t.elapsed() < SETTLE) {
            return None;
        }
        if std::mem::take(&mut pixels.all) {
            pixels.paths.clear();
            pixels.since = None;
            Some(Update::ImagesChangedAll)
        } else if !pixels.paths.is_empty() {
            let paths: Vec<_> = pixels.paths.iter().take(super::BATCH).cloned().collect();
            for p in &paths {
                pixels.paths.remove(p);
            }
            if pixels.paths.is_empty() {
                pixels.since = None;
            }
            Some(Update::ImagesChanged(paths))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn changed(path: PathBuf) -> notify::Result<notify::Event> {
        Ok(notify::Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Any,
        )))
        .add_path(path))
    }
    #[test]
    fn repeated_events_coalesce_and_distinct_path_overflow_requests_full_reconciliation() {
        let revisions = Revisions::default();
        let inbox = ChangeInbox::new(revisions.clone(), Arc::new(|| {}));
        let photo = PathBuf::from("photo.png");
        for _ in 0..10000 {
            inbox.record(changed(photo.clone()));
        }
        assert_eq!(inbox.pending.lock().unwrap().dirty.len(), 1);
        assert_eq!(inbox.pixels.lock().unwrap().paths.len(), 1);
        assert_eq!(inbox.signal.len(), 1);
        for n in 0..MAX_PATHS + 10 {
            inbox.record(changed(PathBuf::from(format!("{n}.png"))));
        }
        let pending = inbox.take();
        assert!(pending.refresh_all && pending.rescan);
        assert!(pending.dirty.is_empty());
        assert!(inbox.pixels.lock().unwrap().all);
        assert!(inbox.pixels.lock().unwrap().paths.is_empty());
        assert!(!revisions.accepts(&super::super::Keywords {
            path: photo,
            tags: Default::default(),
            revision: 0,
            generation: 0
        }));
    }
    #[test]
    fn rescan_and_error_invalidate_old_results_but_preserve_in_progress_user_write() {
        let revisions = Revisions::default();
        let path = PathBuf::from("photo.bmp");
        revisions.bump(&path);
        let inbox = ChangeInbox::new(revisions.clone(), Arc::new(|| {}));
        inbox.record(Err(notify::Error::generic("watcher overflow")));
        let pending = inbox.take();
        assert!(pending.warning.unwrap().contains("watcher overflow"));
        assert!(pending.refresh_all && pending.rescan);
        assert_eq!(revisions.snapshot(&path).0, 1);
        revisions.bump(&path);
        let (revision, generation) = revisions.snapshot(&path);
        assert!(revisions.accepts(&super::super::Keywords {
            path,
            tags: Default::default(),
            revision,
            generation
        }));
    }
    #[test]
    fn operating_system_rescan_flag_is_honored_even_on_an_access_event() {
        let inbox = ChangeInbox::new(Revisions::default(), Arc::new(|| {}));
        inbox.record(Ok(notify::Event::new(EventKind::Access(
            notify::event::AccessKind::Any,
        ))
        .set_flag(notify::event::Flag::Rescan)));
        let pending = inbox.take();
        assert!(pending.refresh_all && pending.rescan);
        assert!(inbox.pixels.lock().unwrap().all);
    }
    #[test]
    fn burst_deadline_is_bounded_and_stopped_mailboxes_ignore_future_events() {
        let inbox = ChangeInbox::new(Revisions::default(), Arc::new(|| {}));
        let path = PathBuf::from("a.jpg");
        inbox.record(changed(path.clone()));
        let first = inbox.pending.lock().unwrap().since;
        for _ in 0..100 {
            inbox.record(changed(path.clone()));
        }
        assert_eq!(inbox.pending.lock().unwrap().since, first);
        assert!(inbox.take_pixels().is_none());
        inbox.stop();
        inbox.take();
        inbox.record(changed(PathBuf::from("ignored.jpg")));
        assert!(inbox.delay().is_none());
    }
}

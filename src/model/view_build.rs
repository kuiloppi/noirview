//! Cooperative view construction, including navigation indexes. One logical item
//! per step; the caller supplies both a time slice and an item ceiling.
use super::*;
use crossbeam_channel::{bounded, Sender, TrySendError};
use std::collections::VecDeque;

#[derive(Clone, Copy)]
enum Phase {
    Filter,
    History,
    Pool,
    Seen,
    Done,
}
pub struct ViewBuild {
    filter: Filter,
    reset: bool,
    preferred: Option<PathBuf>,
    fallback: Option<usize>,
    phase: Phase,
    cursor: usize,
    before: usize,
    visible: VisibleFiles,
    nav: Navigator,
}
pub struct BuiltView {
    pub visible: VisibleFiles,
    pub nav: Navigator,
    pub current: Option<PathBuf>,
    pub reset: bool,
}
impl ViewBuild {
    pub fn new(
        filter: Filter,
        reset: bool,
        preferred: Option<PathBuf>,
        fallback: Option<usize>,
    ) -> Self {
        Self {
            filter,
            reset,
            preferred,
            fallback,
            phase: Phase::Filter,
            cursor: 0,
            before: 0,
            visible: VisibleFiles::default(),
            nav: Navigator::default(),
        }
    }
    pub fn step(&mut self, files: &[PathBuf], photos: &Photos, previous: &Navigator) -> bool {
        match self.phase {
            Phase::Filter => {
                if let Some(path) = files.get(self.cursor) {
                    if self
                        .filter
                        .matches(photos.get(path).unwrap_or(&PhotoMeta::default()))
                    {
                        let i = self.visible.paths.len();
                        self.visible.positions.insert(path.clone(), i);
                        self.visible.paths.push(path.clone());
                    }
                    self.cursor += 1;
                } else {
                    let retained = self
                        .preferred
                        .as_ref()
                        .is_some_and(|p| self.visible.position(p).is_some());
                    if !retained {
                        self.preferred = self
                            .visible
                            .paths
                            .get(
                                self.fallback
                                    .unwrap_or(0)
                                    .min(self.visible.len().saturating_sub(1)),
                            )
                            .cloned();
                        self.reset = true;
                    }
                    self.nav.cycle = previous.cycle;
                    self.nav.history_limit = previous.history_limit;
                    if self.reset {
                        if let Some(p) = &self.preferred {
                            self.nav.history.push_back(p.clone());
                            self.nav.seen.insert(p.clone(), previous.cycle);
                        }
                        self.phase = Phase::Pool;
                    } else {
                        self.nav.backward = previous.backward;
                        self.nav.future_order = previous.future_order;
                        self.phase = Phase::History;
                    }
                    self.cursor = 0;
                }
            }
            Phase::History => {
                if let Some(p) = previous.history.get(self.cursor) {
                    if self.visible.position(p).is_some() {
                        self.nav.history.push_back(p.clone());
                        if self.cursor <= previous.cursor {
                            self.before += 1;
                        }
                    }
                    self.cursor += 1;
                } else {
                    self.nav.cursor = self.before.saturating_sub(1);
                    self.phase = Phase::Pool;
                    self.cursor = 0;
                }
            }
            Phase::Pool => {
                if let Some(p) = self.visible.paths.get(self.cursor) {
                    self.nav.remaining.push(self.cursor);
                    self.nav.slots.push(self.cursor);
                    if !self.reset {
                        if let Some(cycle) = previous.seen.get(p) {
                            self.nav.seen.insert(p.clone(), *cycle);
                        }
                    }
                    self.cursor += 1;
                } else {
                    self.nav.remaining_len = self.visible.len();
                    self.phase = Phase::Seen;
                    self.cursor = 0;
                }
            }
            Phase::Seen => {
                if let Some(p) = self.visible.paths.get(self.cursor) {
                    if self.nav.seen.get(p) == Some(&self.nav.cycle) {
                        self.nav.remove_candidate(self.cursor);
                    }
                    self.cursor += 1;
                } else {
                    self.phase = Phase::Done;
                }
            }
            Phase::Done => return true,
        }
        matches!(self.phase, Phase::Done)
    }
    pub fn finish(self) -> BuiltView {
        assert!(matches!(self.phase, Phase::Done));
        BuiltView {
            visible: self.visible,
            nav: self.nav,
            current: self.preferred,
            reset: self.reset,
        }
    }
}

/// Large obsolete maps/paths are freed by one worker, including cancelled builds.
/// Two queued and two pending objects, plus the active drop. Producers pause view
/// replacement/folder delivery when this allowance is full; never block the GUI.
pub struct Retire {
    tx: Sender<Box<dyn Send>>,
    pending: VecDeque<Box<dyn Send>>,
}
impl Retire {
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = bounded::<Box<dyn Send>>(2);
        std::thread::spawn(move || {
            while let Ok(item) = rx.recv() {
                drop(item);
                wake();
            }
        });
        Self {
            tx,
            pending: VecDeque::new(),
        }
    }
    pub fn pump(&mut self) {
        while let Some(item) = self.pending.pop_front() {
            match self.tx.try_send(item) {
                Ok(()) => {}
                Err(TrySendError::Full(item) | TrySendError::Disconnected(item)) => {
                    self.pending.push_front(item);
                    break;
                }
            }
        }
    }
    pub fn available(&self) -> bool {
        self.pending.len() < 2
    }
    pub fn retire(&mut self, item: impl Send + 'static) {
        assert!(self.available());
        self.pending.push_back(Box::new(item));
        self.pump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(
        mut job: ViewBuild,
        files: &[PathBuf],
        photos: &Photos,
        previous: &Navigator,
    ) -> BuiltView {
        let mut steps = 0;
        while !job.step(files, photos, previous) {
            steps += 1;
            assert!(steps < files.len() * 5 + previous.history.len() + 20);
        }
        job.finish()
    }
    #[test]
    fn cooperative_filter_and_navigation_match_existing_retain_including_shuffle_history() {
        let paths: Vec<PathBuf> = (0..5000).map(|n| format!("photo-{n}.jpg").into()).collect();
        let visible: VisibleFiles = paths.clone().into();
        let mut nav = Navigator::default();
        nav.reset(&visible, Some(&paths[0]));
        let mut current = Some(paths[0].clone());
        for _ in 0..120 {
            current = nav.next(&visible, current.as_deref(), Order::Shuffle);
        }
        for _ in 0..9 {
            current = nav.previous(&visible, current.as_deref(), Order::Shuffle);
        }
        let mut photos = Photos::default();
        for (i, p) in paths.iter().enumerate() {
            photos.entry(p.clone()).or_default().favorite =
                i % 3 == 0 || Some(p) == current.as_ref();
        }
        let filter = Filter {
            favorite: true,
            ..Default::default()
        };
        let built = run(
            ViewBuild::new(filter.clone(), false, current.clone(), None),
            &paths,
            &photos,
            &nav,
        );
        let expected: VisibleFiles = paths
            .iter()
            .filter(|p| filter.matches(&photos[p.as_path()]))
            .cloned()
            .collect::<Vec<_>>()
            .into();
        nav.retain(&expected);
        assert_eq!(built.visible.paths, expected.paths);
        assert_eq!(built.nav.history, nav.history);
        assert_eq!(built.nav.future_order, nav.future_order);
        assert!(built.nav.future.is_empty() && built.nav.past.is_empty());
        assert_eq!(built.nav.cursor, nav.cursor);
        assert_eq!(built.nav.seen, nav.seen);
        assert_eq!(built.nav.remaining, nav.remaining);
        assert_eq!(built.nav.slots, nav.slots);
        assert_eq!(built.nav.remaining_len, nav.remaining_len);
        assert_eq!(built.current, current);
    }
    #[test]
    fn cancelled_filter_can_be_replaced_without_publishing_partial_or_stale_rows() {
        let paths: Vec<PathBuf> = (0..100_000).map(|n| format!("{n}.jpg").into()).collect();
        let mut photos = Photos::default();
        photos.entry(paths[99_999].clone()).or_default().elite = true;
        let nav = Navigator::default();
        let mut stale = ViewBuild::new(Filter::default(), true, None, None);
        for _ in 0..17 {
            assert!(!stale.step(&paths, &photos, &nav));
        }
        let result = run(
            ViewBuild::new(
                Filter {
                    elite: true,
                    ..Default::default()
                },
                true,
                None,
                None,
            ),
            &paths,
            &photos,
            &nav,
        );
        assert_eq!(result.visible.paths, vec![paths[99_999].clone()]);
        assert_eq!(result.current, Some(paths[99_999].clone()));
        assert_eq!(result.nav.remaining_len, 0);
        drop(stale);
        // The new folder and an empty result cannot retain old indexes/history.
        let result = run(
            ViewBuild::new(
                Filter {
                    favorite: true,
                    ..Default::default()
                },
                true,
                None,
                None,
            ),
            &["other.jpg".into()],
            &Photos::default(),
            &result.nav,
        );
        assert!(result.visible.is_empty());
        assert!(result.current.is_none());
        assert!(result.nav.history.is_empty());
    }
    #[test]
    fn removed_current_uses_the_old_position_and_single_image_rollover_remains_live() {
        let paths = vec![PathBuf::from("a"), PathBuf::from("c")];
        let mut result = run(
            ViewBuild::new(Filter::default(), false, Some("b".into()), Some(1)),
            &paths,
            &Photos::default(),
            &Navigator::default(),
        );
        assert_eq!(result.current, Some("c".into()));
        assert!(result.reset);
        assert_eq!(
            result.nav.previous(
                &result.visible,
                Some(std::path::Path::new("c")),
                Order::Sequential
            ),
            Some("a".into())
        );
        let paths = vec![PathBuf::from("one")];
        let mut result = run(
            ViewBuild::new(Filter::default(), true, None, None),
            &paths,
            &Photos::default(),
            &Navigator::default(),
        );
        for _ in 0..10 {
            assert_eq!(
                result.nav.next(
                    &result.visible,
                    Some(std::path::Path::new("one")),
                    Order::Shuffle
                ),
                Some("one".into())
            );
        }
    }
    #[test]
    fn reclamation_backpressure_never_blocks_the_producer() {
        use std::time::Duration;
        struct Block {
            wait: crossbeam_channel::Receiver<()>,
        }
        impl Drop for Block {
            fn drop(&mut self) {
                self.wait.recv().unwrap();
            }
        }
        let (release, wait) = bounded(1);
        let (woke, wake) = bounded(8);
        let mut retire = Retire::new(move || {
            let _ = woke.send(());
        });
        retire.retire(Block { wait });
        // A running destructor can block indefinitely; queued/pending admission is finite.
        for _ in 0..4 {
            if retire.available() {
                retire.retire(vec![0u8; 1024]);
            }
        }
        assert!(!retire.available());
        release.send(()).unwrap();
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while !retire.available() {
            retire.pump();
            assert!(std::time::Instant::now() < end);
            std::thread::yield_now();
        }
    }
}

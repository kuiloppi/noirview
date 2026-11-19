//! Read-only bootstrap. No writer or backup may start before this completes.
use crate::{model::Library, storage::Store};
use crossbeam_channel::{bounded, Receiver};
pub struct Loaded {
    pub store: Option<Store>,
    pub library: Result<Library, String>,
}
pub struct Startup(Receiver<Loaded>);
impl Startup {
    pub fn new(wake: impl FnOnce() + Send + 'static) -> Self {
        Self::with_reader(
            || match Store::discover() {
                Ok(store) => {
                    let library = store.load().map_err(|e| format!("{e:#}"));
                    Loaded {
                        store: Some(store),
                        library,
                    }
                }
                Err(e) => Loaded {
                    store: None,
                    library: Err(format!("{e:#}")),
                },
            },
            wake,
        )
    }
    pub(crate) fn with_reader(
        read: impl FnOnce() -> Loaded + Send + 'static,
        wake: impl FnOnce() + Send + 'static,
    ) -> Self {
        let (tx, rx) = bounded(1);
        std::thread::spawn(move || {
            // Dropping the receiver abandons delivery; never join a blocked read on close.
            if tx.send(read()).is_ok() {
                wake();
            }
        });
        Self(rx)
    }
    pub fn poll(&self) -> Option<Loaded> {
        self.0.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn blocked_bootstrap_is_nonblocking_and_preserves_real_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            path: dir.path().join("library.json"),
            exe_dir: dir.path().into(),
        };
        let mut lib = Library {
            schema: 1,
            ..Default::default()
        };
        lib.settings.prefetch_ahead = 13;
        lib.last_image = Some("image.jpg".into());
        lib.photos.entry("image.jpg".into()).or_default().favorite = true;
        store.save(&lib).unwrap();
        let before = std::fs::read(crate::state::database_path(&store)).unwrap();
        let (release, wait) = bounded(1);
        let (woke, wake) = bounded(1);
        let load = Startup::with_reader(
            move || {
                wait.recv().unwrap();
                let library = store.load().map_err(|e| e.to_string());
                Loaded {
                    store: Some(store),
                    library,
                }
            },
            move || {
                woke.send(()).unwrap();
            },
        );
        for _ in 0..100 {
            assert!(load.poll().is_none());
        }
        release.send(()).unwrap();
        wake.recv_timeout(Duration::from_secs(5)).unwrap();
        let loaded = load.poll().unwrap();
        let lib = loaded.library.unwrap();
        assert!(lib.photos[std::path::Path::new("image.jpg")].favorite);
        assert_eq!(lib.settings.prefetch_ahead, 13);
        assert_eq!(lib.last_image, Some("image.jpg".into()));
        assert_eq!(
            before,
            std::fs::read(crate::state::database_path(&loaded.store.unwrap())).unwrap()
        );
    }
    #[test]
    fn closing_during_a_blocked_read_abandons_delivery_without_waking_or_writing() {
        let (release, wait) = bounded(1);
        let (done, finished) = bounded(1);
        let load = Startup::with_reader(
            move || {
                wait.recv().unwrap();
                done.send(()).unwrap();
                Loaded {
                    store: None,
                    library: Err("unreadable original preserved".into()),
                }
            },
            || panic!("abandoned bootstrap must not wake"),
        );
        drop(load);
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

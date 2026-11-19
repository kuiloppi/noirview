use super::*;
use crate::identity::{Observation, Snapshot};
use crossbeam_channel::{unbounded, Receiver};
use std::{fs, sync::Arc};
fn setup() -> (
    tempfile::TempDir,
    Store,
    PathBuf,
    Library,
    StateWriter,
    Receiver<Notice>,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.jpg");
    fs::write(&path, include_bytes!("../tests/fixtures/iptc-keywords.jpg")).unwrap();
    let store = Store {
        path: dir.path().join("data/library.json"),
        exe_dir: dir.path().into(),
    };
    let mut lib = Library {
        schema: 1,
        ..Default::default()
    };
    lib.photos.entry(path.clone()).or_default().favorite = true;
    lib.photos.entry(path.clone()).or_default().elite = true;
    store.save(&lib).unwrap();
    let lib = store.load().unwrap();
    assert!(lib.photos[&path].identity.is_some());
    let (tx, rx) = unbounded();
    let writer = StateWriter::new(store.clone(), move |n| {
        let _ = tx.send(n);
    });
    (dir, store, path, lib, writer, rx)
}
fn flush(w: &mut StateWriter, lib: &mut Library, rx: &Receiver<Notice>) {
    w.flush(lib).unwrap();
    match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        Notice::Saved(r, true) => r.unwrap(),
        _ => panic!("expected flush"),
    }
}
fn snapshot(paths: &[&Path]) -> Arc<Snapshot> {
    Arc::new(Snapshot {
        serial: 0,
        paths: paths
            .iter()
            .map(|p| {
                (
                    p.to_path_buf(),
                    Observation {
                        identity: identity::capture(p),
                    },
                )
            })
            .collect(),
    })
}
#[test]
fn offline_rename_persists_reconciled_owner_across_real_writer_reopen() {
    let (dir, store, a, mut lib, mut w, rx) = setup();
    let b = dir.path().join("renamed.jpg");
    fs::rename(&a, &b).unwrap();
    w.identify(&mut lib, snapshot(&[&b]), 9).unwrap();
    match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        Notice::Identified(Ok(p), 9) => {
            assert_eq!(p.relocated, 1);
            p.apply(&mut lib).unwrap();
        }
        _ => panic!("expected identity plan"),
    }
    flush(&mut w, &mut lib, &rx);
    w.shutdown();
    let reopened = store.load().unwrap();
    assert!(reopened.photos[&b].elite);
    assert!(!reopened.photos.contains_key(&a));
    assert_eq!(reopened.tracked_marks.iter().next().unwrap().1.path, b);
}
#[test]
fn path_reuse_archives_survive_sql_backup_restore_then_find_original() {
    let (dir, store, a, mut lib, mut w, rx) = setup();
    let b = dir.path().join("outside.jpg");
    let old = identity::capture(&a).unwrap();
    fs::rename(&a, &b).unwrap();
    fs::write(&a, b"new object").unwrap();
    identity::plan(&lib, snapshot(&[&a]))
        .apply(&mut lib)
        .unwrap();
    flush(&mut w, &mut lib, &rx);
    w.shutdown();
    let mut lib = store.load().unwrap();
    assert!(!lib.photos[&a].favorite);
    assert!(lib.tracked_marks.get(&old).unwrap().elite);
    let backup = store.backup(&lib).unwrap();
    lib.tracked_marks.set(old.clone(), None);
    store.save(&lib).unwrap();
    let mut restored = store.restore(&backup, &lib).unwrap();
    identity::plan(&restored, snapshot(&[&b]))
        .apply(&mut restored)
        .unwrap();
    restored.sync_marks(None);
    store.save(&restored).unwrap();
    let reopened = store.load().unwrap();
    assert!(reopened.photos[&b].elite);
    assert!(!reopened.photos[&a].favorite);
}
#[test]
fn metadata_replacement_rebinds_marks_and_same_object_rebind_does_not_erase_them() {
    let (_dir, store, a, mut lib, mut w, rx) = setup();
    let old = identity::capture(&a).unwrap();
    crate::metadata::write(&a, &std::collections::BTreeSet::from(["changed".into()])).unwrap();
    let new = identity::capture(&a).unwrap();
    assert_ne!(old, new);
    lib.rebind_file(&a);
    lib.photos
        .entry(a.clone())
        .or_default()
        .tags
        .insert("changed".into());
    flush(&mut w, &mut lib, &rx);
    let db = store.load().unwrap();
    assert!(db.photos[&a].elite);
    assert_eq!(db.photos[&a].identity.as_deref(), Some(new.as_str()));
    assert!(db.tracked_marks.get(&old).is_none());
    assert!(db.tracked_marks.get(&new).unwrap().elite);
    lib = db;
    lib.rebind_file(&a);
    flush(&mut w, &mut lib, &rx);
    w.shutdown();
    assert!(store.load().unwrap().tracked_marks.get(&new).unwrap().elite);
}
#[test]
fn explicit_internal_move_retains_marks_with_new_object_and_with_rename() {
    for copy in [true, false] {
        let (dir, store, a, mut lib, mut w, rx) = setup();
        let b = dir.path().join("a-first.jpg");
        if copy {
            fs::copy(&a, &b).unwrap();
            fs::remove_file(&a).unwrap();
        } else {
            fs::rename(&a, &b).unwrap();
        }
        let mut m = lib.photos.remove(&a).unwrap();
        m.rebound_from = Some(m.identity.take().unwrap_or_default());
        lib.photos.insert(b.clone(), m);
        flush(&mut w, &mut lib, &rx);
        w.shutdown();
        let db = store.load().unwrap();
        let id = identity::capture(&b).unwrap();
        assert!(db.photos[&b].elite);
        assert_eq!(db.photos[&b].identity.as_ref(), Some(&id));
        assert_eq!(db.tracked_marks.get(&id).unwrap().path, b);
    }
}
#[test]
fn unmark_and_explicit_deletion_do_not_resurrect_after_rename() {
    let (dir, store, a, mut lib, mut w, rx) = setup();
    let id = identity::capture(&a).unwrap();
    let m = lib.photos.entry(a.clone()).or_default();
    m.favorite = false;
    m.elite = false;
    flush(&mut w, &mut lib, &rx);
    let b = dir.path().join("renamed.jpg");
    fs::rename(&a, &b).unwrap();
    lib = store.load().unwrap();
    assert!(lib.tracked_marks.get(&id).is_none());
    let p = identity::plan(&lib, snapshot(&[&b]));
    assert_eq!(p.relocated, 0);
    p.apply(&mut lib).unwrap();
    assert!(!lib.photos.get(&b).is_some_and(|m| m.favorite));
    lib.photos.remove(&a);
    flush(&mut w, &mut lib, &rx);
    w.shutdown();
    assert!(store.load().unwrap().tracked_marks.get(&id).is_none());
}
#[test]
fn optional_tables_upgrade_old_sql_without_touching_legacy_bytes() {
    let (_dir, store, a, lib, w, _) = setup();
    w.shutdown();
    let raw = b"legacy preserved";
    fs::write(&store.path, raw).unwrap();
    let c = Connection::open(database_path(&store)).unwrap();
    c.execute_batch("DROP TABLE image_identity; DROP TABLE file_marks;")
        .unwrap();
    drop(c);
    let mut old = store.load().unwrap();
    assert!(old.photos[&a].identity.is_none());
    assert_eq!(old.photos[&a].favorite, lib.photos[&a].favorite);
    let (tx, rx) = unbounded();
    let mut w = StateWriter::new(store.clone(), move |n| {
        let _ = tx.send(n);
    });
    old.photos.entry(a.clone()).or_default().favorite = true;
    flush(&mut w, &mut old, &rx);
    w.shutdown();
    assert!(store.load().unwrap().photos[&a].identity.is_some());
    assert_eq!(fs::read(&store.path).unwrap(), raw);
}
#[test]
fn mark_ownership_and_photo_changes_rollback_together_on_sql_failure() {
    let (_dir, store, a, mut lib, mut w, rx) = setup();
    let c = Connection::open(database_path(&store)).unwrap();
    c.execute_batch("CREATE TRIGGER reject_marks BEFORE DELETE ON file_marks BEGIN SELECT RAISE(ABORT,'test ownership failure'); END;").unwrap();
    let m = lib.photos.entry(a.clone()).or_default();
    m.favorite = false;
    m.elite = false;
    w.flush(&mut lib).unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(10)).unwrap(),
        Notice::Saved(Err(_), true)
    ));
    let unchanged = store.load().unwrap();
    assert!(unchanged.photos[&a].elite);
    assert!(unchanged.tracked_marks.iter().next().unwrap().1.elite);
    c.execute_batch("DROP TRIGGER reject_marks;").unwrap();
    flush(&mut w, &mut lib, &rx);
    w.shutdown();
    let db = store.load().unwrap();
    assert!(!db.photos[&a].elite);
    assert_eq!(db.tracked_marks.iter().count(), 0);
}
#[test]
fn coalescing_later_mark_edits_retains_pending_metadata_rebinding() {
    for worker_merge in [false, true] {
        let (_dir, store, a, mut lib, w, _) = setup();
        w.shutdown();
        let old = identity::capture(&a).unwrap();
        crate::metadata::write(&a, &std::collections::BTreeSet::from(["new".into()])).unwrap();
        let new = identity::capture(&a).unwrap();
        lib.rebind_file(&a);
        let mut pending = Patch::default();
        pending.capture(&mut lib);
        lib.photos.entry(a.clone()).or_default().elite = false;
        if worker_merge {
            let mut newer = Patch::default();
            newer.capture(&mut lib);
            pending.merge(newer);
        } else {
            pending.capture(&mut lib);
        }
        let mut connection = open(&store).unwrap();
        pending.apply(&mut connection).unwrap();
        drop(connection);
        let result = store.load().unwrap();
        assert_eq!(result.photos[&a].identity.as_ref(), Some(&new));
        assert!(result.photos[&a].favorite);
        assert!(!result.photos[&a].elite);
        assert!(result.tracked_marks.get(&old).is_none());
    }
}

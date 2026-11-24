//! Scale measurements of the user library: load at startup, saves and backup.
use super::*;
use crate::scale_bench::{self as bench, mib, ms, report};
use std::time::Instant;

#[test]
#[ignore = "Scale measurement; writes NOIRVIEW_BENCH_SIZES rows under NOIRVIEW_BENCH_DIR"]
fn scale_bench_user_state() {
    const NAME: &str = "user_state";
    for count in bench::sizes() {
        let dir = bench::workspace("state");
        let store = Store {
            path: dir.path().join("library.json"),
            exe_dir: dir.path().into(),
        };
        let mut lib = Library {
            schema: 1,
            ..Default::default()
        };
        for index in 0..count {
            let tags = bench::tags(index);
            let favorite = index % 50 == 0;
            if !tags.is_empty() || favorite {
                lib.tags.extend(tags.iter().cloned());
                lib.photos.insert(
                    bench::virtual_path(index),
                    PhotoMeta {
                        tags,
                        favorite,
                        elite: index % 500 == 0,
                        ..Default::default()
                    },
                );
            }
        }
        lib.photos.clear_changes();
        let records = lib.photos.len();
        report(NAME, count, "image_records", records as f64, "rows");

        // Create the schema first so the timed write is one pass, as in a restore.
        store.save(&Library::default()).unwrap();
        let started = Instant::now();
        store.save(&lib).unwrap();
        report(
            NAME,
            count,
            "full_replace_as_in_restore",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "database_size",
            mib(std::fs::metadata(database_path(&store)).unwrap().len() as usize),
            "MiB",
        );

        let before = bench::live();
        let started = Instant::now();
        let loaded = store.load().unwrap();
        report(NAME, count, "startup_load", ms(started.elapsed()), "ms");
        report(
            NAME,
            count,
            "loaded_library_heap",
            mib(bench::live() - before),
            "MiB",
        );
        assert_eq!(loaded.photos.len(), records);
        drop(loaded);

        let mut connection = open(&store).unwrap();
        lib.photos
            .entry(bench::virtual_path(0))
            .or_default()
            .favorite = true;
        let mut patch = Patch::default();
        let started = Instant::now();
        patch.capture(&mut lib);
        patch.apply(&mut connection).unwrap();
        report(
            NAME,
            count,
            "one_image_delta_save",
            ms(started.elapsed()),
            "ms",
        );
        drop(connection);

        // The application first rebuilds the library from SQLite (see
        // `startup_load`) and then serializes it, so add both for a real backup.
        let base = bench::reset_peak();
        let started = Instant::now();
        let backup = store.backup(&lib).unwrap();
        report(
            NAME,
            count,
            "backup_serialize_and_write",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "backup_extra_peak_heap",
            mib(bench::peak() - base),
            "MiB",
        );
        report(
            NAME,
            count,
            "backup_file_size",
            mib(std::fs::metadata(backup).unwrap().len() as usize),
            "MiB",
        );
    }
}

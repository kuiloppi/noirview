//! Scale measurements of folder opening, the keyword cache and keyword delivery.
//! See `crate::scale_bench` for how to run them.
use super::*;
use crate::scale_bench::{self as bench, mib, ms, report};
use std::{fs, sync::atomic::AtomicBool};

struct Opened {
    files: Duration,
    identities: Option<Duration>,
    indexing_started: Duration,
    total: Duration,
    progress: Progress,
    peak: usize,
}
/// One complete open of a recursive folder with the real metadata reader.
/// Result payloads are dropped; the application would retain them.
fn open(folder: &Path, database: &Path) -> Opened {
    let active = AtomicBool::new(true);
    let marks = Mutex::new((None, None, None, None));
    let revisions = Revisions::default();
    let changes = Arc::new(ChangeInbox::new(revisions.clone(), Arc::new(|| {})));
    let base = bench::reset_peak();
    let started = Instant::now();
    run(
        folder.to_path_buf(),
        true,
        Some(database.to_path_buf()),
        false,
        &|update| {
            let mut marks = marks.lock().unwrap();
            match update {
                Update::Files(_) => {
                    marks.0.get_or_insert(started.elapsed());
                }
                Update::Identities(_) => {
                    marks.1.get_or_insert(started.elapsed());
                }
                Update::Progress(_) => {
                    marks.2.get_or_insert(started.elapsed());
                }
                Update::Complete(progress) => {
                    marks.3 = Some(progress);
                    active.store(false, Ordering::Relaxed);
                }
                Update::Warning(message) => println!("warning: {message}"),
                Update::Error(message) => panic!("{message}"),
                _ => {}
            }
            true
        },
        &|| active.load(Ordering::Relaxed),
        &revisions,
        changes,
        metadata::read,
    );
    let total = started.elapsed();
    let marks = marks.into_inner().unwrap();
    Opened {
        files: marks.0.expect("file list"),
        identities: marks.1,
        indexing_started: marks.2.expect("index start"),
        total,
        progress: marks.3.expect("index completion"),
        peak: bench::peak() - base,
    }
}
#[test]
#[ignore = "Scale measurement; creates NOIRVIEW_BENCH_SIZES files under NOIRVIEW_BENCH_DIR"]
fn scale_bench_folder_open() {
    const NAME: &str = "folder_open";
    for count in bench::sizes() {
        let work = bench::workspace("index");
        let tree = work.path().join("images");
        let started = Instant::now();
        bench::write_tree(&tree, count);
        report(
            NAME,
            count,
            "setup_create_files",
            ms(started.elapsed()),
            "ms",
        );

        let before = bench::live();
        let started = Instant::now();
        let files = model::scan(&tree, true).unwrap().paths;
        report(NAME, count, "scan", ms(started.elapsed()), "ms");
        report(
            NAME,
            count,
            "scan_result_heap",
            mib(bench::live() - before),
            "MiB",
        );
        assert_eq!(files.len(), count);

        // The worker builds the path set that the UI receives with the list.
        let before = bench::live();
        let started = Instant::now();
        let snapshot = FileSnapshot::new(&files);
        report(
            NAME,
            count,
            "file_snapshot_build",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "file_snapshot_heap",
            mib(bench::live() - before),
            "MiB",
        );
        drop(snapshot);

        // Identity capture opens every file. It runs on each open and again after
        // every burst of filesystem changes.
        let before = bench::live();
        let started = Instant::now();
        let kept = Mutex::new(None);
        // A first open with nothing remembered: every file is opened.
        assert!(send_identities(
            &files,
            &mut HashMap::new(),
            &|_| false,
            &mut Vec::new(),
            &Revisions::default(),
            &|| true,
            &|update| {
                *kept.lock().unwrap() = Some(update);
                true
            }
        ));
        report(
            NAME,
            count,
            "identity_snapshot",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "identity_snapshot_heap",
            mib(bench::live() - before),
            "MiB",
        );
        drop(kept);
        drop(files);

        let database = work.path().join("keyword-index.sqlite3");
        let cold = open(&tree, &database);
        assert_eq!(cold.progress.read, count);
        assert_eq!(cold.progress.failed, 0);
        report(NAME, count, "cold_file_list", ms(cold.files), "ms");
        report(
            NAME,
            count,
            "cold_metadata_read",
            ms(cold.progress.elapsed),
            "ms",
        );
        report(NAME, count, "cold_total", ms(cold.total), "ms");
        report(NAME, count, "cold_worker_peak_heap", mib(cold.peak), "MiB");
        report(
            NAME,
            count,
            "index_database_size",
            // The rows live in the catalog beside the given path (0.1.79).
            mib([database.clone(), KeywordIndex::catalog_path(&database)]
                .iter()
                .filter_map(|p| fs::metadata(p).ok())
                .map(|m| m.len() as usize)
                .sum()),
            "MiB",
        );

        let warm = open(&tree, &database);
        assert_eq!(warm.progress.cached, count);
        assert_eq!(warm.progress.read, 0);
        report(NAME, count, "warm_file_list", ms(warm.files), "ms");
        report(
            NAME,
            count,
            "warm_cache_replay_and_identities",
            ms(warm.indexing_started - warm.files),
            "ms",
        );
        if let Some(at) = warm.identities {
            report(NAME, count, "warm_identities_delivered_at", ms(at), "ms");
        }
        report(
            NAME,
            count,
            "warm_fingerprint_validation",
            ms(warm.progress.elapsed),
            "ms",
        );
        report(NAME, count, "warm_total", ms(warm.total), "ms");
        report(NAME, count, "warm_worker_peak_heap", mib(warm.peak), "MiB");

        {
            let mut index = KeywordIndex::open(&database).unwrap();
            // The folder worker's step after a scan (C1: reconcile the listing;
            // before, pruning), with nothing changed.
            let scanned = model::scan(&tree, true).unwrap();
            let started = Instant::now();
            index
                .reconcile(
                    &tree,
                    true,
                    &scanned.paths,
                    &scanned.listed,
                    &scanned.created,
                )
                .unwrap();
            report(
                NAME,
                count,
                "cache_prune_nothing_removed",
                ms(started.elapsed()),
                "ms",
            );
            let before = bench::live();
            let started = Instant::now();
            let rows = index.load_any(&tree, true).unwrap();
            report(NAME, count, "cache_load_any", ms(started.elapsed()), "ms");
            report(
                NAME,
                count,
                "cache_load_heap",
                mib(bench::live() - before),
                "MiB",
            );
            assert_eq!(rows.len(), count);
        }

        fs::write(
            bench::tree_path(&tree, 0),
            bench::jpeg(&BTreeSet::from(["changed for the benchmark".into()])),
        )
        .unwrap();
        let changed = open(&tree, &database);
        assert_eq!(changed.progress.read, 1);
        report(
            NAME,
            count,
            "one_changed_file_total",
            ms(changed.total),
            "ms",
        );
    }
}
#[test]
#[ignore = "Scale measurement of the UI-thread keyword merge; no filesystem access"]
fn scale_bench_apply_keywords() {
    const NAME: &str = "apply_keywords";
    for count in bench::sizes() {
        let files: Vec<_> = (0..count).map(bench::virtual_path).collect();
        let paths: HashSet<_> = files.iter().cloned().collect();
        let loader = FolderLoader::new(|| {});
        let mut lib = Library::default();
        let batches = || -> Vec<Vec<Keywords>> {
            files
                .chunks(BATCH)
                .enumerate()
                .map(|(chunk, batch)| {
                    batch
                        .iter()
                        .enumerate()
                        .map(|(offset, path)| Keywords {
                            path: path.clone(),
                            tags: bench::tags(chunk * BATCH + offset),
                            revision: 0,
                            generation: 0,
                        })
                        .collect()
                })
                .collect()
        };
        for (metric, expect_change) in [("first_delivery", true), ("unchanged_replay", false)] {
            let before = bench::live();
            let input = batches();
            let started = Instant::now();
            let mut changed = false;
            for batch in input {
                changed |= apply_keywords(&mut lib, &paths, &loader, batch);
            }
            let elapsed = started.elapsed();
            assert_eq!(changed, expect_change);
            report(NAME, count, metric, ms(elapsed), "ms");
            report(
                NAME,
                count,
                &format!("{metric}_per_4ms_slice"),
                count as f64 * 0.004 / elapsed.as_secs_f64(),
                "images",
            );
            if expect_change {
                report(
                    NAME,
                    count,
                    "library_records_heap",
                    mib(bench::live().saturating_sub(before)),
                    "MiB",
                );
            }
        }
    }
}

//! Scale measurements of the catalog foundation. See `crate::scale_bench`.
use super::*;
use crate::scale_bench::{self as bench, mib, ms, report};
use std::time::Instant;

/// Sorted-list intersection, as a filter over several keywords would combine them.
fn intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}
#[test]
#[ignore = "Scale measurement; writes NOIRVIEW_BENCH_SIZES rows under NOIRVIEW_BENCH_DIR"]
fn scale_bench_catalog() {
    const NAME: &str = "catalog";
    for count in bench::sizes() {
        let dir = bench::workspace("catalog");
        let database = dir.path().join("catalog.sqlite3");
        let root = bench::virtual_path(0)
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf();
        let rows: Vec<_> = (0..count)
            .map(|index| {
                (
                    bench::virtual_path(index),
                    Entry {
                        image: Stamp {
                            size: 1000 + index as u64,
                            modified_ns: Some(1_700_000_000_000_000_000 + index as i64),
                        },
                        sidecar: None,
                        tags: bench::tags(index),
                    },
                )
            })
            .collect();
        // Larger batches first, in a database of their own: the initial pass over
        // a new folder could use them, and the difference shows what each commit costs.
        {
            let large = dir.path().join("catalog-large-batches.sqlite3");
            let mut catalog = Catalog::open(&large).unwrap();
            let started = Instant::now();
            for batch in rows.chunks(1024) {
                catalog.store(batch).unwrap();
            }
            report(
                NAME,
                count,
                "store_all_in_batches_of_1024",
                ms(started.elapsed()),
                "ms",
            );
        }
        {
            // Batches of 64, as the indexing worker flushes them today.
            let mut catalog = Catalog::open(&database).unwrap();
            let started = Instant::now();
            for batch in rows.chunks(64) {
                catalog.store(batch).unwrap();
            }
            report(
                NAME,
                count,
                "store_all_in_batches_of_64",
                ms(started.elapsed()),
                "ms",
            );
            let started = Instant::now();
            catalog.checkpoint().unwrap();
            report(
                NAME,
                count,
                "checkpoint_after_store",
                ms(started.elapsed()),
                "ms",
            );
        }
        report(
            NAME,
            count,
            "database_size",
            mib(std::fs::metadata(&database).unwrap().len() as usize),
            "MiB",
        );
        let mut catalog = Catalog::open(&database).unwrap();

        // Opening one folder of 1,000 images inside the whole collection.
        let album = bench::virtual_path(count / 2)
            .parent()
            .unwrap()
            .to_path_buf();
        let started = Instant::now();
        let listed = catalog.ids(&album, Depth::Children).unwrap();
        report(
            NAME,
            count,
            "list_one_folder_ids",
            ms(started.elapsed()),
            "ms",
        );
        let started = Instant::now();
        let loaded = catalog.load(&album, Depth::Children).unwrap();
        report(
            NAME,
            count,
            "load_one_folder_entries",
            ms(started.elapsed()),
            "ms",
        );
        assert_eq!(listed.len(), loaded.len());
        assert_eq!(listed.len(), 1000.min(count - count / 2 / 1000 * 1000));

        // The whole collection, as a recursive open of the root needs it.
        let before = bench::live();
        let started = Instant::now();
        let all = catalog.ids(&root, Depth::Subtree).unwrap();
        report(
            NAME,
            count,
            "list_all_ids_with_paths",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "list_all_ids_heap",
            mib(bench::live() - before),
            "MiB",
        );
        assert_eq!(all.len(), count);
        drop(all);
        let before = bench::live();
        let started = Instant::now();
        let everything = catalog.load(&root, Depth::Subtree).unwrap();
        report(NAME, count, "load_all_entries", ms(started.elapsed()), "ms");
        report(
            NAME,
            count,
            "load_all_entries_heap",
            mib(bench::live() - before),
            "MiB",
        );
        assert_eq!(everything.len(), count);
        drop(everything);

        // Keyword filters answered from the relation, as id lists.
        let before = bench::live();
        let started = Instant::now();
        let common = catalog.ids_with_keyword(bench::COMMON_TAG).unwrap();
        report(
            NAME,
            count,
            "ids_with_common_keyword",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "common_keyword_matches",
            common.len() as f64,
            "images",
        );
        report(
            NAME,
            count,
            "common_keyword_ids_heap",
            mib(bench::live() - before),
            "MiB",
        );
        let started = Instant::now();
        let second = catalog.ids_with_keyword("kw0001").unwrap();
        let both = intersect(&common, &second);
        report(
            NAME,
            count,
            "second_keyword_and_intersection",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "two_keyword_matches",
            both.len() as f64,
            "images",
        );

        let started = Instant::now();
        for index in (0..count).step_by((count / 1000).max(1)) {
            assert!(catalog.get(&bench::virtual_path(index)).unwrap().is_some());
        }
        report(
            NAME,
            count,
            "get_1000_single_images",
            ms(started.elapsed()),
            "ms",
        );

        // One changed image: the cost of keeping the catalog current.
        let started = Instant::now();
        catalog.store(&rows[count / 3..count / 3 + 1]).unwrap();
        report(NAME, count, "store_one_image", ms(started.elapsed()), "ms");

        let valid: HashSet<_> = loaded.keys().cloned().collect();
        let started = Instant::now();
        assert_eq!(catalog.retain(&album, Depth::Children, &valid).unwrap(), 0);
        report(
            NAME,
            count,
            "retain_one_folder_nothing_removed",
            ms(started.elapsed()),
            "ms",
        );
    }
}

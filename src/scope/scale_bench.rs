//! Scale measurements of the compact lists. See `crate::scale_bench`.
use super::*;
use crate::{
    catalog::{Catalog, Depth, Entry, Stamp},
    scale_bench::{self as bench, mib, ms, report},
};
use std::time::Instant;

#[test]
#[ignore = "Scale measurement; writes NOIRVIEW_BENCH_SIZES rows under NOIRVIEW_BENCH_DIR"]
fn scale_bench_scope() {
    const NAME: &str = "scope";
    for count in bench::sizes() {
        let dir = bench::workspace("scope");
        let mut catalog = Catalog::open(&dir.path().join("catalog.sqlite3")).unwrap();
        // Camera-like names: the same thousand file names repeat in every folder,
        // so name order interleaves the folders and every name has many ties.
        let path = |index: usize| {
            bench::virtual_path(index)
                .parent()
                .unwrap()
                .join(format!("IMG_{:04}.jpg", index % 1000))
        };
        let rows: Vec<_> = (0..count)
            .map(|index| {
                (
                    path(index),
                    Entry {
                        image: Stamp {
                            size: 1,
                            modified_ns: Some(1),
                        },
                        sidecar: None,
                        tags: bench::tags(index),
                    },
                )
            })
            .collect();
        for batch in rows.chunks(1024) {
            catalog.store(batch).unwrap();
        }
        drop(rows);
        let root = path(0)
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf();

        let before = bench::live();
        let started = Instant::now();
        let scope =
            Arc::new(Scope::build(catalog.listing(&root, Depth::Subtree).unwrap()).unwrap());
        report(
            NAME,
            count,
            "scope_build_from_catalog",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "scope_heap",
            mib(bench::live() - before),
            "MiB",
        );
        assert_eq!(scope.len(), count);

        let before = bench::live();
        let base = bench::reset_peak();
        let started = Instant::now();
        let all = View::by_name(scope.clone(), None, false);
        report(NAME, count, "view_by_name_all", ms(started.elapsed()), "ms");
        report(NAME, count, "view_heap", mib(bench::live() - before), "MiB");
        report(
            NAME,
            count,
            "view_build_peak_heap",
            mib(bench::peak() - base),
            "MiB",
        );
        assert_eq!(all.len(), count);

        // One keyword filter from query to finished view.
        let started = Instant::now();
        let ids = catalog.ids_with_keyword(bench::COMMON_TAG).unwrap();
        let shown = ImageSet::filter(
            scope.len(),
            crate::model::FilterMode::All,
            &[scope.set_of_ids(&ids)],
        );
        let filtered = View::by_name(scope.clone(), Some(&shown), false);
        report(
            NAME,
            count,
            "keyword_filter_to_view",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "keyword_filter_matches",
            filtered.len() as f64,
            "images",
        );

        let started = Instant::now();
        for index in (0..count).step_by((count / 1000).max(1)) {
            assert!(all.position_of_path(&path(index)).is_some());
        }
        report(
            NAME,
            count,
            "position_of_1000_paths",
            ms(started.elapsed()),
            "ms",
        );
        let started = Instant::now();
        let page = all.page(count / 2, 64);
        report(
            NAME,
            count,
            "one_page_of_64_paths",
            ms(started.elapsed()),
            "ms",
        );
        assert_eq!(page.len(), 64);

        // The unfavourable case for name order: every file name is different and
        // unordered, so each one needs its own natural comparison.
        let listing: crate::catalog::Listing = (0..count.div_ceil(1000))
            .map(|album| {
                let folder = crate::catalog::key(path(album * 1000).parent().unwrap());
                let images = (album * 1000..count.min(album * 1000 + 1000))
                    .map(|index| {
                        let scrambled = (index as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                        (index as u32, format!("{scrambled:016x}.jpg").into_bytes())
                    })
                    .collect();
                (folder, images)
            })
            .collect();
        let unique = Arc::new(Scope::build(listing).unwrap());
        let started = Instant::now();
        let view = View::by_name(unique.clone(), None, false);
        report(
            NAME,
            count,
            "view_by_name_all_unique_names",
            ms(started.elapsed()),
            "ms",
        );
        assert_eq!(view.len(), count);
        drop(view);
        drop(unique);

        // A rescan replaced the scope: carry every image number over.
        let again = Scope::build(catalog.listing(&root, Depth::Subtree).unwrap()).unwrap();
        let started = Instant::now();
        let kept = (0..count as u32)
            .filter(|number| translate(&scope, &again, *number).is_some())
            .count();
        report(
            NAME,
            count,
            "translate_all_numbers",
            ms(started.elapsed()),
            "ms",
        );
        assert_eq!(kept, count);
    }
}

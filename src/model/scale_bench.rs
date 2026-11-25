//! Scale measurements of whole-list work: sorting, view construction and grid
//! paging. `Viewer` needs a live GUI context, so these call the same functions
//! `Viewer::sort_files`, `Viewer::poll_view` and the grid use, on the same types.
use super::{view_build::BuiltView, *};
use crate::scale_bench::{self as bench, mib, ms, report};
use std::time::{Duration, Instant};

const NAME: &str = "ui_list";

/// Run one view build to completion the way `Viewer::poll_view` does in frames
/// without input (`view_build::slice`). Returns total working time and the
/// slice count.
fn build(
    files: &[PathBuf],
    photos: &Photos,
    filter: Filter,
    previous: &Navigator,
    reset: bool,
) -> (BuiltView, Duration, usize) {
    let started = Instant::now();
    let mut view = ViewBuild::new(filter, reset, files.first().cloned(), None);
    let mut slices = 0;
    loop {
        slices += 1;
        let (time, steps) = super::view_build::slice(false);
        let deadline = Instant::now() + time;
        let mut done = false;
        for _ in 0..steps {
            done = view.step(files, photos, previous);
            if done || Instant::now() >= deadline {
                break;
            }
        }
        if done {
            return (view.finish(), started.elapsed(), slices);
        }
    }
}
#[test]
#[ignore = "Scale measurement of list sorting, view building and grid paging; no filesystem access"]
fn scale_bench_view_lists() {
    for count in bench::sizes() {
        let start = bench::live();
        let mut files: Vec<_> = (0..count).map(bench::virtual_path).collect();
        report(
            NAME,
            count,
            "file_list_heap",
            mib(bench::live() - start),
            "MiB",
        );

        // `Viewer::sort_files` runs this on the UI thread when the list arrives.
        let started = Instant::now();
        crate::sorting::sort(&mut files, crate::sorting::SortKey::Name, false);
        report(
            NAME,
            count,
            "sort_by_name_ordered_input",
            ms(started.elapsed()),
            "ms",
        );
        // The synthetic names are already in order, which is the best case. A
        // recursive folder arrives ordered by full path, not by file name.
        let mut unordered = files.clone();
        rand::seq::SliceRandom::shuffle(unordered.as_mut_slice(), &mut rand::thread_rng());
        let started = Instant::now();
        crate::sorting::sort(&mut unordered, crate::sorting::SortKey::Name, false);
        report(
            NAME,
            count,
            "sort_by_name_unordered_input",
            ms(started.elapsed()),
            "ms",
        );
        assert_eq!(unordered, files);
        drop(unordered);

        let before = bench::live();
        let mut photos = Photos::default();
        for (index, path) in files.iter().enumerate() {
            let tags = bench::tags(index);
            if !tags.is_empty() {
                photos.insert(
                    path.clone(),
                    PhotoMeta {
                        tags,
                        ..Default::default()
                    },
                );
            }
        }
        photos.clear_changes();
        report(
            NAME,
            count,
            "photo_records_heap",
            mib(bench::live() - before),
            "MiB",
        );

        let before = bench::live();
        let empty = Navigator::default();
        let (all, time, slices) = build(&files, &photos, Filter::default(), &empty, true);
        assert_eq!(all.visible.len(), count);
        report(NAME, count, "view_build_unfiltered_total", ms(time), "ms");
        report(
            NAME,
            count,
            "view_build_unfiltered_slices",
            slices as f64,
            "frames",
        );
        report(
            NAME,
            count,
            "visible_and_navigator_heap",
            mib(bench::live() - before),
            "MiB",
        );
        report(
            NAME,
            count,
            "ui_structures_heap_without_path_set",
            mib(bench::live() - start),
            "MiB",
        );

        let filter = Filter {
            tags: BTreeSet::from([bench::COMMON_TAG.into()]),
            ..Default::default()
        };
        let (matching, time, slices) = build(&files, &photos, filter.clone(), &all.nav, true);
        report(NAME, count, "view_build_tag_filter_total", ms(time), "ms");
        report(
            NAME,
            count,
            "view_build_tag_filter_slices",
            slices as f64,
            "frames",
        );
        report(
            NAME,
            count,
            "tag_filter_matches",
            matching.visible.len() as f64,
            "images",
        );

        // A changed keyword batch with an active filter restarts this build.
        let (_, time, _) = build(&files, &photos, filter, &matching.nav, false);
        report(
            NAME,
            count,
            "view_rebuild_per_keyword_change",
            ms(time),
            "ms",
        );

        // First Shuffle grid page: the whole visible list is copied and shuffled.
        let mut pages = crate::grid_pages::Pages::default();
        pages.sync(&all.visible, 12, Order::Shuffle);
        let before = bench::live();
        let started = Instant::now();
        let (page, _) = pages.current(&all.visible, 12, Order::Shuffle);
        report(
            NAME,
            count,
            "grid_shuffle_first_page",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            count,
            "grid_shuffle_heap",
            mib(bench::live() - before),
            "MiB",
        );
        assert_eq!(page.len(), 12.min(count));
    }
}

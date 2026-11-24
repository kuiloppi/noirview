//! Shared helpers for the ignored `scale_bench_*` measurements. Test builds only.
//!
//! Run every measurement with:
//!
//! ```text
//! cargo test --release -- --ignored scale_bench --nocapture --test-threads=1
//! ```
//!
//! `NOIRVIEW_BENCH_SIZES` is a comma-separated list of collection sizes (default
//! `10000,100000`). `NOIRVIEW_BENCH_DIR` selects the drive on which file trees and
//! databases are created (default: the system temporary directory), so the same
//! command can be pointed at an SSD or an HDD.
//!
//! Every result is printed as `BENCH | bench | size | metric | value | unit` and is
//! also appended to the file named by `NOIRVIEW_BENCH_OUT`, if set.
//! Heap figures are bytes requested from the allocator by this process. They exclude
//! allocator bookkeeping and fragmentation, so resident memory is somewhat higher.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}
// SAFETY: every request is forwarded unchanged to the system allocator; the
// counters are only statistics and never influence the returned memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, size) };
        if !moved.is_null() {
            if size >= layout.size() {
                grow(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        moved
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

/// Heap bytes currently requested by the test process.
pub fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}
/// Start a new peak measurement and return the current live bytes as its base.
pub fn reset_peak() -> usize {
    let live = live();
    PEAK.store(live, Ordering::Relaxed);
    live
}
pub fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}
pub fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024. * 1024.)
}
pub fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.
}
/// Print one result and, when `NOIRVIEW_BENCH_OUT` names a file, append it there.
/// The file is what `tools/measure.ps1` collects: an optimized Windows build has no
/// console, so standard output alone would be lost.
pub fn report(bench: &str, size: usize, metric: &str, value: f64, unit: &str) {
    use std::io::Write;
    let line = format!("BENCH | {bench} | {size} | {metric} | {value:.3} | {unit}");
    println!("{line}");
    if let Some(path) = std::env::var_os("NOIRVIEW_BENCH_OUT") {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path);
        if let Ok(mut file) = file {
            let _ = writeln!(file, "{line}");
        }
    }
}
pub fn sizes() -> Vec<usize> {
    std::env::var("NOIRVIEW_BENCH_SIZES")
        .unwrap_or_else(|_| "10000,100000".into())
        .split(',')
        .filter_map(|size| size.trim().replace('_', "").parse().ok())
        .filter(|size| *size > 0)
        .collect()
}
/// A scratch directory on the drive selected by `NOIRVIEW_BENCH_DIR`.
pub fn workspace(name: &str) -> tempfile::TempDir {
    let prefix = format!("noirview-bench-{name}-");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix);
    match std::env::var_os("NOIRVIEW_BENCH_DIR") {
        Some(root) => {
            std::fs::create_dir_all(&root).unwrap();
            builder.tempdir_in(root).unwrap()
        }
        None => builder.tempdir().unwrap(),
    }
}

pub const VOCABULARY: usize = 2000;
/// The most frequent keyword; about a fifth of all images carry it.
pub const COMMON_TAG: &str = "kw0000";
const PER_FOLDER: usize = 1000;

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
/// Deterministic keywords for image `index`: 30 % of images have none, the others
/// carry one to eight keywords drawn from a skewed 2,000-word vocabulary.
pub fn tags(index: usize) -> BTreeSet<String> {
    let mut state = mix(index as u64);
    if state % 10 < 3 {
        return BTreeSet::new();
    }
    let count = 1 + (state >> 8) % 8;
    (0..count)
        .map(|_| {
            state = mix(state);
            let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
            format!("kw{:04}", (VOCABULARY as f64 * unit.powi(3)) as usize)
        })
        .collect()
}
/// Album folder and file name, joined with the platform's separator: paths are
/// compared as bytes, so they must be spelled the way a directory walk spells them.
fn relative(index: usize) -> PathBuf {
    Path::new(&format!("album-{:05}", index / PER_FOLDER)).join(format!("IMG_{index:08}.jpg"))
}
/// A path that is never touched on disk, for list/database measurements.
pub fn virtual_path(index: usize) -> PathBuf {
    Path::new(if cfg!(windows) { "C:\\" } else { "/" })
        .join("Photos")
        .join("archive")
        .join(relative(index))
}
/// The smallest file `metadata::read` accepts as a JPEG: start marker, an optional
/// XMP segment with `dc:subject`, end marker. It contains no pixels.
pub fn jpeg(tags: &BTreeSet<String>) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8];
    if !tags.is_empty() {
        let mut xmp = String::from(
            "<x:xmpmeta xmlns:x='adobe:ns:meta/'>\
             <rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'>\
             <rdf:Description xmlns:dc='http://purl.org/dc/elements/1.1/'>\
             <dc:subject><rdf:Bag>",
        );
        for tag in tags {
            xmp.push_str(&format!("<rdf:li>{tag}</rdf:li>"));
        }
        xmp.push_str("</rdf:Bag></dc:subject></rdf:Description></rdf:RDF></x:xmpmeta>");
        let header = b"http://ns.adobe.com/xap/1.0/\0";
        out.extend_from_slice(&[0xff, 0xe1]);
        out.extend_from_slice(&((2 + header.len() + xmp.len()) as u16).to_be_bytes());
        out.extend_from_slice(header);
        out.extend_from_slice(xmp.as_bytes());
    }
    out.extend_from_slice(&[0xff, 0xd9]);
    out
}
/// Create `count` tiny JPEG files, 1,000 per subfolder, with `tags(index)` embedded.
pub fn write_tree(root: &Path, count: usize) {
    for index in 0..count {
        if index % PER_FOLDER == 0 {
            std::fs::create_dir_all(root.join(format!("album-{:05}", index / PER_FOLDER))).unwrap();
        }
        std::fs::write(root.join(relative(index)), jpeg(&tags(index))).unwrap();
    }
}
pub fn tree_path(root: &Path, index: usize) -> PathBuf {
    root.join(relative(index))
}

#[test]
fn synthetic_images_are_readable_and_keywords_are_skewed() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(dir.path(), 2000);
    let mut tagged = 0;
    let mut common = 0;
    for index in 0..2000 {
        let read = crate::metadata::read(&tree_path(dir.path(), index)).unwrap();
        assert_eq!(read, tags(index));
        tagged += usize::from(!read.is_empty());
        common += usize::from(read.contains(COMMON_TAG));
    }
    assert!((1300..1500).contains(&tagged), "{tagged}");
    assert!((300..700).contains(&common), "{common}");
    assert_eq!(
        crate::model::scan(dir.path(), true).unwrap().paths.len(),
        2000
    );
}

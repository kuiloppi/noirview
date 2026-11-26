//! Per-image cost of decoding and of the persistent thumbnail cache. These do not
//! depend on collection size; multiply by the number of images a grid or a
//! library needs.
use super::*;
use crate::scale_bench::{self as bench, mib, ms, report};
use std::time::{Duration, Instant};

fn request(path: PathBuf, thumb: bool) -> Request {
    Request {
        key: Key {
            path,
            thumb,
            preview: false,
            revision: 0,
        },
        settings: Settings::default(),
        max_side: 8192,
        cancelled: AtomicBool::new(false),
        unwanted: AtomicBool::new(false),
    }
}
#[test]
#[ignore = "Decode and thumbnail-cache cost on synthetic 12 and 24 megapixel JPEGs"]
fn scale_bench_decode_and_thumbnail_cache() {
    const NAME: &str = "decode";
    let dir = bench::workspace("decode");
    for (width, height) in [(4000u32, 3000u32), (6000, 4000)] {
        let pixels = (width * height) as usize;
        let path = dir.path().join(format!("{width}x{height}.jpg"));
        // Smooth structure plus fine noise, so the file is neither trivially
        // compressible nor pure noise.
        let source = image::RgbImage::from_fn(width, height, |x, y| {
            let noise = (x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca6b)) >> 28;
            let wave = ((x as f32 / 97.).sin() * (y as f32 / 61.).cos() * 60.) as i32;
            let level = |base: u32| (base as i32 + wave + noise as i32).clamp(0, 255) as u8;
            image::Rgb([
                level(x * 200 / width),
                level(y * 200 / height),
                level((x + y) * 100 / (width + height)),
            ])
        });
        image::codecs::jpeg::JpegEncoder::new_with_quality(
            std::io::BufWriter::new(std::fs::File::create(&path).unwrap()),
            90,
        )
        .encode_image(&source)
        .unwrap();
        drop(source);
        report(
            NAME,
            pixels,
            "source_file_size",
            mib(std::fs::metadata(&path).unwrap().len() as usize),
            "MiB",
        );
        for (metric, thumb) in [("thumbnail_from_source", true), ("full_image", false)] {
            let mut samples = vec![];
            for _ in 0..5 {
                let r = request(path.clone(), thumb);
                let started = Instant::now();
                let (rgba, dimensions, _) = decode(&r, &Mutex::new(None), None).unwrap();
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                );
                samples.push(started.elapsed());
                assert_eq!(dimensions, [width, height]);
                assert_eq!(color.size[0] <= 512, thumb);
            }
            samples.sort_unstable();
            report(NAME, pixels, metric, ms(samples[2]), "ms");
        }

        // Persistent cache: first generation, stored size and a later hit.
        let cache = Arc::new(ThumbnailCache::new(
            Some(dir.path().join(format!("cache-{pixels}"))),
            512,
        ));
        let started = Instant::now();
        let outcome = generate_thumbnail(&cache, &path, &Settings::default());
        assert_eq!(outcome, crate::pregen::Outcome::Generated);
        cache.wait_idle(Duration::from_secs(30)).unwrap();
        report(
            NAME,
            pixels,
            "thumbnail_generate_and_store",
            ms(started.elapsed()),
            "ms",
        );
        report(
            NAME,
            pixels,
            "thumbnail_stored_size",
            cache.stats().bytes as f64 / 1024.,
            "KiB",
        );
        let mut samples = vec![];
        for _ in 0..5 {
            let started = Instant::now();
            let token = cache.token(&path, Kind::Thumbnail).unwrap();
            let hit = cache.load(&token, |_| true).unwrap();
            samples.push(started.elapsed());
            assert!(hit.is_some());
        }
        samples.sort_unstable();
        report(NAME, pixels, "thumbnail_cache_hit", ms(samples[2]), "ms");
    }
    // Storing many thumbnails in a row: what the writer costs per image once the
    // decode is cheap. Small sources keep decoding out of the picture.
    const SMALL: usize = 300;
    let small = dir.path().join("small");
    std::fs::create_dir_all(&small).unwrap();
    let paths: Vec<PathBuf> = (0..SMALL)
        .map(|index| {
            let path = small.join(format!("{index:04}.jpg"));
            image::RgbImage::from_fn(800, 600, |x, y| {
                image::Rgb([(x + index as u32) as u8, (y * 2) as u8, (x ^ y) as u8])
            })
            .save(&path)
            .unwrap();
            path
        })
        .collect();
    let cache = Arc::new(ThumbnailCache::new(
        Some(dir.path().join("cache-small")),
        512,
    ));
    let started = Instant::now();
    for path in &paths {
        assert_eq!(
            generate_thumbnail(&cache, path, &Settings::default()),
            crate::pregen::Outcome::Generated
        );
    }
    let generated = started.elapsed();
    cache.wait_idle(Duration::from_secs(120)).unwrap();
    let stats = cache.stats();
    report(
        NAME,
        SMALL,
        "small_thumbnail_generate_each",
        ms(generated) / SMALL as f64,
        "ms",
    );
    report(
        NAME,
        SMALL,
        "small_thumbnail_generate_and_store_each",
        ms(started.elapsed()) / SMALL as f64,
        "ms",
    );
    report(
        NAME,
        SMALL,
        "small_thumbnails_stored",
        stats.entries as f64,
        "entries",
    );
    report(
        NAME,
        SMALL,
        "small_thumbnails_skipped",
        stats.skipped as f64,
        "entries",
    );
}

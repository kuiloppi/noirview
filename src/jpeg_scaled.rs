//! Reduced-IDCT JPEG path for disposable display previews/grid tiles only.
//! Full image processing continues through imaging's existing source decoder.
use crate::decode_stats::{self, Phase};
use anyhow::{ensure, Context, Result};
use image::{DynamicImage, GrayImage, RgbImage, RgbaImage};
use jpeg_decoder::{CodingProcess, Decoder, PixelFormat};
use std::{
    io::{Cursor, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
const MAX_INPUT: u64 = 64 * 1024 * 1024;
const MAX_DECODED: usize = 32 * 1024 * 1024;
const MAX_WORKING: usize = 256 * 1024 * 1024;
/// Whether the first frame header is progressive (SOF2/6/10/14).
fn progressive(data: &[u8]) -> bool {
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xff {
            return false;
        }
        let marker = data[at + 1];
        if marker == 0xff {
            at += 1;
            continue;
        }
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            return matches!(marker, 0xc2 | 0xc6 | 0xca | 0xce);
        }
        if marker == 0xda {
            return false;
        }
        at += 2 + u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
    }
    false
}
struct Cancellable<'a, R> {
    reader: R,
    cancelled: &'a AtomicBool,
}
impl<R: Read> Read for Cancellable<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("cancelled"));
        }
        self.reader.read(buffer)
    }
}
fn target(dimensions: [u32; 2], cap: u32) -> [u32; 2] {
    let [w, h] = dimensions;
    let side = w.max(h);
    if side <= cap {
        return dimensions;
    }
    [
        ((w as u64 * cap as u64 + side as u64 / 2) / side as u64).max(1) as u32,
        ((h as u64 * cap as u64 + side as u64 / 2) / side as u64).max(1) as u32,
    ]
}
fn working_bound(
    original: [u32; 2],
    scaled: [u32; 2],
    channels: usize,
    progressive: bool,
    input: u64,
) -> Option<usize> {
    let [w, h] = scaled;
    // Decoder planes + packed output + RGBA conversion/resampling coexist. Add
    // compressed-file-sized metadata allowance and a conservative MCU-row bound.
    let decoded = (w as usize)
        .checked_mul(h as usize)?
        .checked_mul(channels)?;
    let rgba = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    let aligned_w = (original[0] as usize).checked_add(31)? / 32 * 32;
    let aligned_h = (original[1] as usize).checked_add(31)? / 32 * 32;
    let coefficients = if progressive {
        aligned_w
            .checked_mul(aligned_h)?
            .checked_mul(channels)?
            .checked_mul(2)?
    } else {
        0
    };
    decoded
        .checked_mul(2)?
        .checked_add(rgba.checked_mul(2)?)?
        .checked_add(coefficients)?
        .checked_add(input as usize)?
        .checked_add(aligned_w.checked_mul(channels)?.checked_mul(128)?)?
        .checked_add(1024 * 1024)
}
/// Reduced-IDCT decode (`jpeg-decoder`) at the smallest DCT scale that covers
/// the requested size.
fn decode_reduced(
    data: &[u8],
    original: [u32; 2],
    requested: [u32; 2],
    budget: usize,
    cancelled: &AtomicBool,
    reserve: &mut impl FnMut(usize) -> bool,
) -> Result<DynamicImage> {
    let reader = Cancellable {
        reader: Cursor::new(data),
        cancelled,
    };
    let mut decoder = Decoder::new(reader);
    decoder.read_info()?;
    let info = decoder.info().context("JPEG information unavailable")?;
    ensure!(
        [info.width as u32, info.height as u32] == original,
        "Inconsistent JPEG dimensions"
    );
    ensure!(
        matches!(
            info.coding_process,
            CodingProcess::DctSequential | CodingProcess::DctProgressive
        ),
        "JPEG coding process requires the normal decoder"
    );
    ensure!(
        info.pixel_format != PixelFormat::L16,
        "High-bit-depth JPEG requires the normal decoder"
    );
    let (w, h) = decoder.scale(requested[0] as u16, requested[1] as u16)?;
    let scaled = [w as u32, h as u32];
    ensure!(
        scaled != original,
        "JPEG is too small to benefit from reduced IDCT"
    );
    let channels = info.pixel_format.pixel_bytes();
    let decoded = w as usize * h as usize * channels;
    ensure!(
        decoded <= MAX_DECODED,
        "JPEG reduced output exceeds the fast-path limit"
    );
    ensure!(
        working_bound(
            original,
            scaled,
            channels,
            info.coding_process == CodingProcess::DctProgressive,
            data.len() as u64
        )
        .is_some_and(|n| n <= budget),
        "JPEG fast-path working memory limit exceeded"
    );
    ensure!(
        reserve(requested[0] as usize * requested[1] as usize * 4),
        crate::image_queue::PrefetchDeferred
    );
    decoder.set_max_decoding_buffer_size(MAX_DECODED);
    let pixels = decoder.decode()?;
    ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
    ensure!(pixels.len() == decoded, "Unexpected JPEG output length");
    Ok(match info.pixel_format {
        PixelFormat::L8 => DynamicImage::ImageLuma8(
            GrayImage::from_raw(w as u32, h as u32, pixels).context("Invalid grayscale JPEG")?,
        ),
        PixelFormat::RGB24 => DynamicImage::ImageRgb8(
            RgbImage::from_raw(w as u32, h as u32, pixels).context("Invalid RGB JPEG")?,
        ),
        PixelFormat::CMYK32 => {
            let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
            for c in pixels.as_chunks::<4>().0 {
                let k = 255 - c[3] as u16;
                for channel in &c[..3] {
                    rgb.push((((255 - *channel as u16) * k + 127) / 255) as u8);
                }
            }
            DynamicImage::ImageRgb8(
                RgbImage::from_raw(w as u32, h as u32, rgb).context("Invalid CMYK JPEG")?,
            )
        }
        PixelFormat::L16 => unreachable!(),
    })
}
/// Lanczos3 to exactly `size`, SIMD-accelerated (`fast_image_resize`). The
/// same filter as `image`'s Lanczos3; JPEG sources are opaque, so alpha is
/// passed through without premultiplication.
pub fn resize_rgba(image: RgbaImage, size: [u32; 2]) -> Result<RgbaImage> {
    use fast_image_resize::{
        images::Image, FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
    };
    let (w, h) = image.dimensions();
    let source = Image::from_vec_u8(w, h, image.into_raw(), PixelType::U8x4)?;
    let mut target = Image::new(size[0], size[1], PixelType::U8x4);
    Resizer::new().resize(
        &source,
        &mut target,
        &ResizeOptions::new()
            .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
            .use_alpha(false),
    )?;
    RgbaImage::from_raw(size[0], size[1], target.into_vec()).context("Invalid resized image")
}
pub fn decode(
    path: &Path,
    cap: u32,
    budget_mb: usize,
    cancelled: &AtomicBool,
    mut reserve: impl FnMut(usize) -> bool,
) -> Result<(RgbaImage, [u32; 2])> {
    ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
    ensure!((1..=1024).contains(&cap), "Invalid JPEG preview cap");
    let metadata = std::fs::metadata(path)?;
    ensure!(
        metadata.len() <= MAX_INPUT,
        "JPEG exceeds the fast-path input limit"
    );
    // Whole file in one read turn; decoding below runs in parallel from memory.
    let read = Instant::now();
    let data = crate::disk::read_file(path, MAX_INPUT)?;
    decode_stats::add(Phase::Read, read);
    ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
    let mut source = Cursor::new(data.as_slice());
    let (original, orientation) = crate::jpeg_preview::source_info(&mut source)?;
    decode_stats::source(original, progressive(&data));
    let requested = target(original, cap);
    let budget = (budget_mb.clamp(64, 8192) * 1024 * 1024 / 2).min(MAX_WORKING);
    let started = Instant::now();
    let image = decode_reduced(&data, original, requested, budget, cancelled, &mut reserve)?;
    decode_stats::add(Phase::Decode, started);
    // Never enlarge a small source. DCT output rounds to block boundaries, then
    // Lanczos reaches the requested pixel cap.
    let resize = Instant::now();
    let mut image = image.into_rgba8();
    if image.dimensions() != (requested[0], requested[1]) {
        image = resize_rgba(image, requested)?;
    }
    let mut image = DynamicImage::ImageRgba8(image);
    image.apply_orientation(orientation);
    decode_stats::add(Phase::Resize, resize);
    let mut original = original;
    if orientation.to_exif() >= 5 {
        original.swap(0, 1);
    }
    ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
    let after = std::fs::metadata(path)?;
    ensure!(
        after.len() == metadata.len() && after.modified().ok() == metadata.modified().ok(),
        "JPEG changed during preview decoding"
    );
    Ok((image.into_rgba8(), original))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::metadata::Orientation;
    use std::{io::Cursor, time::Instant};
    fn source(orientation: u8, little: bool) -> Vec<u8> {
        let image = RgbImage::from_fn(401, 601, |x, y| {
            image::Rgb(match (x < 200, y < 300) {
                (true, true) => [230, 20, 30],
                (false, true) => [20, 220, 30],
                (true, false) => [20, 30, 230],
                (false, false) => [220, 200, 20],
            })
        });
        let mut out = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        let bytes = out.into_inner();
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(if little {
            b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0"
        } else {
            b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01"
        });
        exif.extend(if little {
            [orientation, 0, 0, 0]
        } else {
            [0, orientation, 0, 0]
        });
        exif.extend([0; 4]);
        let mut file = bytes[..2].to_vec();
        file.extend([0xff, 0xe1]);
        file.extend(((exif.len() + 2) as u16).to_be_bytes());
        file.extend(exif);
        file.extend(&bytes[2..]);
        file
    }
    fn run(bytes: &[u8], cap: u32) -> (RgbaImage, [u32; 2]) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.JPG");
        std::fs::write(&path, bytes).unwrap();
        decode(&path, cap, 768, &AtomicBool::new(false), |_| true).unwrap()
    }
    #[test]
    fn source_without_embedded_thumbnail_scales_and_preserves_all_eight_orientations() {
        for little in [false, true] {
            for orientation in 1..=8 {
                let bytes = source(orientation, little);
                let dir = tempfile::tempdir().unwrap();
                let plain = dir.path().join("no-thumb.jpg");
                std::fs::write(&plain, &bytes).unwrap();
                assert!(crate::jpeg_preview::decode(&plain, |_| true).is_err());
                let (actual, original) = run(&bytes, 128);
                let mut reference = image::load_from_memory(&bytes).unwrap();
                reference.apply_orientation(Orientation::from_exif(orientation).unwrap());
                assert_eq!(original, [reference.width(), reference.height()]);
                assert!(actual.width() <= 128 && actual.height() <= 128);
                let reference = reference
                    .resize_exact(
                        actual.width(),
                        actual.height(),
                        image::imageops::FilterType::Lanczos3,
                    )
                    .into_rgba8();
                for (x, y) in [
                    (5, 5),
                    (actual.width() - 6, 5),
                    (5, actual.height() - 6),
                    (actual.width() - 6, actual.height() - 6),
                ] {
                    for c in 0..3 {
                        assert!(
                            (actual[(x, y)][c] as i16 - reference[(x, y)][c] as i16).abs() <= 5,
                            "orientation={orientation}, little={little}"
                        );
                    }
                    assert_eq!(actual[(x, y)][3], 255);
                }
            }
        }
    }
    #[test]
    fn progressive_grayscale_and_cmyk_match_reference_colors_and_geometry() {
        for bytes in [
            include_bytes!("../tests/fixtures/jpeg/progressive.jpg").as_slice(),
            include_bytes!("../tests/fixtures/jpeg/gray.jpg").as_slice(),
            include_bytes!("../tests/fixtures/jpeg/cmyk.jpg").as_slice(),
        ] {
            let (actual, original) = run(bytes, 64);
            let full = image::load_from_memory(bytes).unwrap();
            assert_eq!(original, [full.width(), full.height()]);
            let reference = full
                .resize_exact(
                    actual.width(),
                    actual.height(),
                    image::imageops::FilterType::Lanczos3,
                )
                .into_rgba8();
            let error: u64 = actual
                .pixels()
                .zip(reference.pixels())
                .map(|(a, b)| {
                    (0..3)
                        .map(|c| (a[c] as i16 - b[c] as i16).unsigned_abs() as u64)
                        .sum::<u64>()
                })
                .sum();
            assert!(
                error as f64 / ((actual.width() * actual.height() * 3) as f64) < 4.0,
                "mean error {}",
                error as f64 / (actual.width() * actual.height() * 3) as f64
            );
        }
    }
    #[test]
    fn simd_lanczos_matches_the_previous_resize() {
        let image = image::load_from_memory(&source(1, true))
            .unwrap()
            .into_rgba8();
        for size in [[128, 192], [267, 400], [61, 33]] {
            let reference = DynamicImage::ImageRgba8(image.clone())
                .resize_exact(size[0], size[1], image::imageops::FilterType::Lanczos3)
                .into_rgba8();
            let fast = resize_rgba(image.clone(), size).unwrap();
            assert_eq!(fast.dimensions(), reference.dimensions());
            let error: u64 = fast
                .as_raw()
                .iter()
                .zip(reference.as_raw())
                .map(|(a, b)| a.abs_diff(*b) as u64)
                .sum();
            let mean = error as f64 / fast.as_raw().len() as f64;
            assert!(mean < 1.5, "{size:?}: mean error {mean}");
            assert!(fast.pixels().all(|p| p[3] == 255));
        }
    }
    #[test]
    fn progressive_marker_is_found_before_the_scan() {
        assert!(progressive(include_bytes!(
            "../tests/fixtures/jpeg/progressive.jpg"
        )));
        assert!(!progressive(&source(1, true)));
        assert!(!progressive(b"\xff\xd8\xff"));
    }
    #[test]
    fn budget_admission_and_cancellation_precede_source_pixel_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, source(1, true)).unwrap();
        let error = decode(&path, 128, 768, &AtomicBool::new(false), |_| false).unwrap_err();
        assert!(error.is::<crate::image_queue::PrefetchDeferred>());
        let error = decode(&path, 128, 768, &AtomicBool::new(true), |_| {
            panic!("cancelled job must not reserve")
        })
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }
    #[test]
    fn huge_progressive_coefficients_and_truncated_sources_are_bounded() {
        let mut bytes = include_bytes!("../tests/fixtures/jpeg/progressive.jpg").to_vec();
        let sof = bytes.windows(2).position(|w| w == [0xff, 0xc2]).unwrap();
        bytes[sof + 5..sof + 7].copy_from_slice(&60000u16.to_be_bytes());
        bytes[sof + 7..sof + 9].copy_from_slice(&60000u16.to_be_bytes());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.jpg");
        std::fs::write(&path, bytes).unwrap();
        assert!(decode(&path, 512, 768, &AtomicBool::new(false), |_| panic!(
            "huge header must fail before reservation"
        ))
        .is_err());
        let bytes = source(1, true);
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(decode(&path, 128, 768, &AtomicBool::new(false), |_| true).is_err());
    }
    #[test]
    fn input_size_and_progressive_scratch_limits_fail_before_output_admission() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limited.jpg");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_INPUT + 1).unwrap();
        drop(file);
        assert!(decode(&path, 512, 768, &AtomicBool::new(false), |_| panic!(
            "input limit must precede allocation"
        ))
        .unwrap_err()
        .to_string()
        .contains("input limit"));
        let mut bytes = include_bytes!("../tests/fixtures/jpeg/progressive.jpg").to_vec();
        let sof = bytes.windows(2).position(|w| w == [0xff, 0xc2]).unwrap();
        bytes[sof + 5..sof + 7].copy_from_slice(&4000u16.to_be_bytes());
        bytes[sof + 7..sof + 9].copy_from_slice(&4000u16.to_be_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(decode(&path, 512, 64, &AtomicBool::new(false), |_| panic!(
            "coefficient limit must precede allocation"
        ))
        .unwrap_err()
        .to_string()
        .contains("working memory"));
        let cancelled = AtomicBool::new(false);
        let mut reader = Cancellable {
            reader: Cursor::new([1, 2, 3]),
            cancelled: &cancelled,
        };
        let mut byte = [0];
        reader.read_exact(&mut byte).unwrap();
        cancelled.store(true, Ordering::Relaxed);
        assert!(reader.read_exact(&mut byte).is_err());
    }
    #[test]
    #[ignore = "opt-in preparation timing; set NOIRVIEW_BENCH_JPEG to an independent JPEG"]
    fn jpeg_preparation_timing() {
        let path = std::env::var_os("NOIRVIEW_BENCH_JPEG").expect("set NOIRVIEW_BENCH_JPEG");
        let path = Path::new(&path);
        let caps = if std::env::var_os("NOIRVIEW_BENCH_CAP512").is_some() {
            vec![512]
        } else {
            vec![512, 1024]
        };
        for cap in caps {
            let start = Instant::now();
            let (image, original) =
                decode(path, cap, 768, &AtomicBool::new(false), |_| true).unwrap();
            eprintln!(
                "scaled JPEG cap={cap} {:?} -> {:?}: {:.3}s",
                original,
                image.dimensions(),
                start.elapsed().as_secs_f64()
            );
            let start = Instant::now();
            let full = image::open(path)
                .unwrap()
                .resize(cap, cap, image::imageops::FilterType::Lanczos3)
                .into_rgba8();
            eprintln!(
                "full-decode/Lanczos cap={cap} -> {:?}: {:.3}s",
                full.dimensions(),
                start.elapsed().as_secs_f64()
            );
        }
    }
}
#[cfg(test)]
mod bench {
    #[test]
    #[ignore = "manual benchmark: NOIRVIEW_BENCH_DIR=<folder of JPEGs>"]
    fn bench_scaled_decode() {
        let Ok(dir) = std::env::var("NOIRVIEW_BENCH_DIR") else {
            return;
        };
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        // Files are read once first so decoding is timed from the OS cache.
        for f in &files {
            let _ = std::fs::read(f);
        }
        let all = std::time::Instant::now();
        for f in &files {
            let t = std::time::Instant::now();
            let r = super::decode(f, 512, 768, &cancelled, |_| true);
            eprintln!(
                "{:?} {:?} {:?}",
                f.file_name().unwrap(),
                t.elapsed(),
                r.map(|r| r.1)
            );
        }
        eprintln!("total {:?}", all.elapsed());
        eprintln!("{:?}", crate::decode_stats::take());
    }
    /// Synthetic photo-like JPEGs (wallpaper and camera sizes, baseline and
    /// progressive): whole thumbnail preparation and the resize step alone,
    /// previous `image` Lanczos3 against the SIMD resize. Runs in Windows CI.
    #[test]
    #[ignore = "manual benchmark: cargo test --release bench_thumbnails -- --ignored --nocapture"]
    fn bench_thumbnails() {
        use image::DynamicImage;
        use std::sync::atomic::AtomicBool;
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let cancelled = AtomicBool::new(false);
        eprintln!("size        kind         MB  thumb ms  old resize  new resize");
        for (w, h) in [(1920usize, 1080usize), (3840, 2160), (6000, 4000)] {
            for progressive in [false, true] {
                let mut files = Vec::new();
                for n in 0..4u32 {
                    let mut seed = 0x9e37_79b9u32 ^ n.wrapping_mul(7919) ^ w as u32;
                    let mut pixels = vec![0u8; w * h * 3];
                    for y in 0..h {
                        for x in 0..w {
                            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            let noise = (seed >> 24) as f32 / 255. * 24. - 12.;
                            let fx = x as f32 / w as f32 * 9. + n as f32;
                            let fy = y as f32 / h as f32 * 7.;
                            let base = [
                                128. + 70. * (fx * 1.3).sin() * (fy * 0.9).cos(),
                                128. + 60. * (fx * 0.7 + fy).cos(),
                                110. + 50. * (fy * 1.7).sin() + 30. * (fx * 3.1).sin(),
                            ];
                            for c in 0..3 {
                                pixels[(y * w + x) * 3 + c] =
                                    (base[c] + noise).clamp(0., 255.) as u8;
                            }
                        }
                    }
                    let path = dir.path().join(format!("{w}x{h}-{progressive}-{n}.jpg"));
                    let mut encoder = jpeg_encoder::Encoder::new_file(&path, 90).unwrap();
                    encoder.set_progressive(progressive);
                    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
                    encoder
                        .encode(&pixels, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
                        .unwrap();
                    files.push(path);
                }
                let mut thumb = 0.;
                for path in &files {
                    let t = Instant::now();
                    super::decode(path, 512, 768, &cancelled, |_| true).unwrap();
                    thumb += t.elapsed().as_secs_f64() * 1000. / files.len() as f64;
                }
                // The resize step alone, from the DCT-scaled size.
                let full = image::open(&files[0]).unwrap();
                let scale = (1..=8)
                    .find(|d| {
                        (w as u32 * d).div_ceil(8) >= 512 && (h as u32 * d).div_ceil(8) >= 288
                    })
                    .unwrap_or(8);
                let scaled = full
                    .resize_exact(
                        (w as u32 * scale).div_ceil(8),
                        (h as u32 * scale).div_ceil(8),
                        image::imageops::FilterType::Triangle,
                    )
                    .into_rgba8();
                let size = [512, (512 * h / w) as u32];
                let t = Instant::now();
                for _ in 0..10 {
                    let _ = DynamicImage::ImageRgba8(scaled.clone()).resize_exact(
                        size[0],
                        size[1],
                        image::imageops::FilterType::Lanczos3,
                    );
                }
                let old = t.elapsed().as_secs_f64() * 100.;
                let t = Instant::now();
                for _ in 0..10 {
                    super::resize_rgba(scaled.clone(), size).unwrap();
                }
                let new = t.elapsed().as_secs_f64() * 100.;
                let mb = files
                    .iter()
                    .map(|f| std::fs::metadata(f).unwrap().len())
                    .sum::<u64>() as f64
                    / files.len() as f64
                    / 1e6;
                eprintln!(
                    "{:>9}  {:<11} {:>4.1} {:>8.1} {:>11.1} {:>11.1}   ({:?} -> {:?})",
                    format!("{w}x{h}"),
                    if progressive {
                        "progressive"
                    } else {
                        "baseline"
                    },
                    mb,
                    thumb,
                    old,
                    new,
                    scaled.dimensions(),
                    size
                );
            }
        }
        eprintln!("{:?}", crate::decode_stats::take());
    }
}

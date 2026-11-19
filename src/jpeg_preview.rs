//! Optional embedded JPEG thumbnails. Probing never reads the main compressed
//! scan or decodes source pixels; absent/invalid thumbnails leave the full path alone.
use anyhow::{ensure, Context, Result};
use image::{metadata::Orientation, DynamicImage, ImageDecoder, RgbaImage};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};

const MAX_HEADER: u64 = 1024 * 1024;
const MAX_SIDE: u32 = 1024;
struct Embedded {
    jpeg: Vec<u8>,
    orientation: u8,
    source_orientation: u8,
    source_size: [u32; 2],
}
#[derive(Default)]
struct Ifd {
    orientation: Option<u8>,
    offset: Option<usize>,
    length: Option<usize>,
    next: usize,
}
fn exif_thumbnail(data: &[u8]) -> Option<Embedded> {
    let tiff = data.strip_prefix(b"Exif\0\0")?;
    let little = match tiff.get(..4)? {
        b"II\x2a\0" => true,
        b"MM\0\x2a" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let bytes = tiff.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let bytes = tiff.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    let ifd = |at: usize| -> Option<Ifd> {
        if at < 8 {
            return None;
        }
        let count = u16_at(at)? as usize;
        let end = at.checked_add(2)?.checked_add(count.checked_mul(12)?)?;
        let mut result = Ifd {
            next: u32_at(end)? as usize,
            ..Default::default()
        };
        for index in 0..count {
            let p = at + 2 + index * 12;
            if u32_at(p + 4)? != 1 {
                continue;
            }
            match (u16_at(p)?, u16_at(p + 2)?) {
                (0x112, 3) => {
                    result.orientation = u8::try_from(u16_at(p + 8)?)
                        .ok()
                        .filter(|n| (1..=8).contains(n))
                }
                (0x201, 4) => result.offset = Some(u32_at(p + 8)? as usize),
                (0x202, 4) => result.length = Some(u32_at(p + 8)? as usize),
                _ => {}
            }
        }
        Some(result)
    };
    let main = ifd(u32_at(4)? as usize)?;
    let thumb = ifd(main.next)?;
    let offset = thumb.offset?;
    let length = thumb.length?;
    if length < 4 || offset < 8 {
        return None;
    }
    let jpeg = tiff.get(offset..offset.checked_add(length)?)?;
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    Some(Embedded {
        jpeg: jpeg.into(),
        orientation: thumb.orientation.or(main.orientation).unwrap_or(1),
        source_orientation: main.orientation.unwrap_or(1),
        source_size: [0, 0],
    })
}
fn scan_header(
    reader: &mut (impl Read + Seek),
) -> Result<(Option<Embedded>, [u32; 2], Orientation)> {
    let mut start = [0; 2];
    reader.read_exact(&mut start)?;
    ensure!(start == [0xff, 0xd8], "Not a JPEG");
    let mut embedded = None;
    let mut dimensions = None;
    let mut orientation = Orientation::NoTransforms;
    for _ in 0..256 {
        ensure!(
            reader.stream_position()? < MAX_HEADER,
            "JPEG preview header limit exceeded"
        );
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        ensure!(byte[0] == 0xff, "Invalid JPEG header");
        // Include fill bytes in the same bounded header budget.
        let mut fill = 0;
        while byte[0] == 0xff {
            fill += 1;
            ensure!(fill <= 256, "JPEG preview marker padding limit exceeded");
            ensure!(
                reader.stream_position()? < MAX_HEADER,
                "JPEG preview header limit exceeded"
            );
            reader.read_exact(&mut byte)?;
        }
        let marker = byte[0];
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        let mut length = [0; 2];
        reader.read_exact(&mut length)?;
        let length = u16::from_be_bytes(length) as usize;
        ensure!(length >= 2, "Invalid JPEG segment length");
        let length = length - 2;
        ensure!(
            reader.stream_position()? + length as u64 <= MAX_HEADER,
            "JPEG preview header limit exceeded"
        );
        let sof = matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf);
        if marker == 0xe1 || sof {
            let mut data = vec![0; length];
            reader.read_exact(&mut data)?;
            if marker == 0xe1 {
                if let Some(tiff) = data.strip_prefix(b"Exif\0\0") {
                    if let Some(value) = Orientation::from_exif_chunk(tiff) {
                        orientation = value;
                    }
                }
                if embedded.is_none() {
                    embedded = exif_thumbnail(&data);
                }
            } else if sof && data.len() >= 6 {
                dimensions = Some([
                    u16::from_be_bytes([data[3], data[4]]) as u32,
                    u16::from_be_bytes([data[1], data[2]]) as u32,
                ]);
            }
        } else {
            reader.seek(SeekFrom::Current(length as i64))?;
        }
    }
    let dimensions = dimensions.context("JPEG source dimensions unavailable")?;
    ensure!(
        dimensions.iter().all(|n| *n > 0),
        "Invalid source dimensions"
    );
    Ok((embedded, dimensions, orientation))
}
fn probe(reader: &mut (impl Read + Seek)) -> Result<Embedded> {
    let (embedded, dimensions, _) = scan_header(reader)?;
    let mut embedded = embedded.context("No embedded JPEG preview")?;
    embedded.source_size = dimensions;
    Ok(embedded)
}
pub(crate) fn source_info(reader: &mut (impl Read + Seek)) -> Result<([u32; 2], Orientation)> {
    let (_, dimensions, orientation) = scan_header(reader)?;
    Ok((dimensions, orientation))
}

pub fn decode(path: &Path, reserve: impl FnOnce(usize) -> bool) -> Result<(RgbaImage, [u32; 2])> {
    let embedded = probe(&mut File::open(path)?)?;
    let mut reader =
        image::ImageReader::with_format(Cursor::new(embedded.jpeg), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(8 * 1024 * 1024);
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    ensure!(
        w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE,
        "Embedded preview dimensions exceed the limit"
    );
    ensure!(
        reserve(w as usize * h as usize * 4),
        crate::image_queue::PrefetchDeferred
    );
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(
        Orientation::from_exif(embedded.orientation).unwrap_or(Orientation::NoTransforms),
    );
    let mut original = embedded.source_size;
    if embedded.source_orientation >= 5 {
        original.swap(0, 1);
    }
    let ratio =
        image.width() as f64 * original[1] as f64 / (image.height() as f64 * original[0] as f64);
    ensure!(
        (0.98..=1.02).contains(&ratio),
        "Embedded preview is cropped or has a different aspect ratio"
    );
    Ok((image.into_rgba8(), original))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(little: bool) -> (Vec<u8>, Vec<u8>) {
        let mut thumb = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 12, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 20) as u8, 90])
        }))
        .write_to(&mut thumb, image::ImageFormat::Jpeg)
        .unwrap();
        let thumb = thumb.into_inner();
        let mut tiff = Vec::new();
        let u16_bytes = |n: u16| {
            if little {
                n.to_le_bytes()
            } else {
                n.to_be_bytes()
            }
        };
        let u32_bytes = |n: u32| {
            if little {
                n.to_le_bytes()
            } else {
                n.to_be_bytes()
            }
        };
        tiff.extend_from_slice(if little { b"II" } else { b"MM" });
        tiff.extend(u16_bytes(42));
        tiff.extend(u32_bytes(8));
        tiff.extend(u16_bytes(1));
        tiff.extend(u16_bytes(0x112));
        tiff.extend(u16_bytes(3));
        tiff.extend(u32_bytes(1));
        tiff.extend(u16_bytes(6));
        tiff.extend([0, 0]);
        tiff.extend(u32_bytes(26));
        tiff.extend(u16_bytes(2));
        for (tag, value) in [(0x201, 56), (0x202, thumb.len() as u32)] {
            tiff.extend(u16_bytes(tag));
            tiff.extend(u16_bytes(4));
            tiff.extend(u32_bytes(1));
            tiff.extend(u32_bytes(value));
        }
        tiff.extend(u32_bytes(0));
        assert_eq!(tiff.len(), 56);
        tiff.extend_from_slice(&thumb);
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend(tiff);
        let mut source = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::new(80, 120))
            .write_to(&mut source, image::ImageFormat::Jpeg)
            .unwrap();
        let source = source.into_inner();
        let mut file = vec![0xff, 0xd8, 0xff, 0xe1];
        file.extend(((payload.len() + 2) as u16).to_be_bytes());
        file.extend(payload);
        file.extend_from_slice(&source[2..]);
        (file, thumb)
    }
    #[test]
    fn embedded_preview_preserves_orientation_for_both_tiff_byte_orders() {
        for little in [true, false] {
            let (file, thumb) = fixture(little);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("source.jpg");
            std::fs::write(&path, file).unwrap();
            let (pixels, original) = decode(&path, |n| {
                assert_eq!(n, 8 * 12 * 4);
                true
            })
            .unwrap();
            assert_eq!(original, [120, 80]);
            assert_eq!(
                pixels,
                image::load_from_memory(&thumb)
                    .unwrap()
                    .rotate90()
                    .into_rgba8()
            );
        }
    }
    #[test]
    fn invalid_or_out_of_bounds_exif_offsets_are_optional_failures() {
        let (file, _) = fixture(true);
        let mut reader = Cursor::new(&file);
        assert!(probe(&mut reader).is_ok());
        let mut invalid = file.clone();
        // IFD1 JPEG offset is TIFF-relative; reject the claimed range before allocation.
        invalid[6 + 6 + 26 + 2 + 8..6 + 6 + 26 + 2 + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(probe(&mut Cursor::new(invalid)).is_err());
        let source = DynamicImage::new_rgb8(80, 120);
        let mut plain = Cursor::new(Vec::new());
        source
            .write_to(&mut plain, image::ImageFormat::Jpeg)
            .unwrap();
        assert!(probe(&mut Cursor::new(plain.into_inner())).is_err());
        let mut padded = Cursor::new([vec![0xff, 0xd8], vec![0xff; 8192]].concat());
        assert!(probe(&mut padded).is_err());
        assert!(padded.position() <= 260);
    }
    #[test]
    fn missing_preview_does_not_read_the_main_compressed_scan() {
        let mut file = vec![0xff, 0xd8, 0xff, 0xda];
        file.resize(4 * 1024 * 1024, 0x55);
        let mut reader = Cursor::new(file);
        assert!(probe(&mut reader).is_err());
        assert_eq!(reader.position(), 4);
    }
    #[test]
    fn oversized_thumbnail_is_rejected_before_its_pixels_are_allocated() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (mut file, _) = fixture(true);
        let sof = file.windows(2).position(|w| w == [0xff, 0xc0]).unwrap();
        file[sof + 7..sof + 9].copy_from_slice(&2000u16.to_be_bytes());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oversized.jpg");
        std::fs::write(&path, file).unwrap();
        let allocated = AtomicBool::new(false);
        assert!(decode(&path, |_| {
            allocated.store(true, Ordering::Relaxed);
            true
        })
        .is_err());
        assert!(!allocated.load(Ordering::Relaxed));
    }
    #[test]
    fn cropped_thumbnail_is_not_stretched_to_the_full_image() {
        let (mut file, _) = fixture(true);
        let sof = file
            .windows(2)
            .enumerate()
            .filter(|(_, w)| *w == [0xff, 0xc0])
            .map(|(i, _)| i)
            .next_back()
            .unwrap();
        file[sof + 7..sof + 9].copy_from_slice(&160u16.to_be_bytes());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cropped.jpg");
        std::fs::write(&path, file).unwrap();
        assert!(decode(&path, |_| true)
            .unwrap_err()
            .to_string()
            .contains("aspect ratio"));
    }
}

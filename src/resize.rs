//! SIMD Lanczos3 (`fast_image_resize`) for decoded images of any common
//! 8-bit layout, without first expanding them to RGBA.
use anyhow::{Context, Result};
use fast_image_resize::{images::Image, FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GrayAlphaImage, GrayImage, RgbImage, RgbaImage};

fn run(
    data: Vec<u8>,
    from: (u32, u32),
    to: [u32; 2],
    pixel: PixelType,
    alpha: bool,
) -> Result<Vec<u8>> {
    let source = Image::from_vec_u8(from.0, from.1, data, pixel)?;
    let mut target = Image::new(to[0], to[1], pixel);
    Resizer::new().resize(
        &source,
        &mut target,
        &ResizeOptions::new()
            .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
            .use_alpha(alpha),
    )?;
    Ok(target.into_vec())
}
/// Lanczos3 to exactly `size`. Transparent pixels are premultiplied for the
/// filter; fully opaque images skip that work.
pub fn lanczos(image: DynamicImage, size: [u32; 2]) -> Result<DynamicImage> {
    let [w, h] = size;
    Ok(match image {
        DynamicImage::ImageRgb8(i) => {
            let from = i.dimensions();
            DynamicImage::ImageRgb8(
                RgbImage::from_raw(w, h, run(i.into_raw(), from, size, PixelType::U8x3, false)?)
                    .context("Invalid resized image")?,
            )
        }
        DynamicImage::ImageRgba8(i) => {
            let from = i.dimensions();
            let alpha = i.pixels().any(|p| p[3] != 255);
            DynamicImage::ImageRgba8(
                RgbaImage::from_raw(w, h, run(i.into_raw(), from, size, PixelType::U8x4, alpha)?)
                    .context("Invalid resized image")?,
            )
        }
        DynamicImage::ImageLuma8(i) => {
            let from = i.dimensions();
            DynamicImage::ImageLuma8(
                GrayImage::from_raw(w, h, run(i.into_raw(), from, size, PixelType::U8, false)?)
                    .context("Invalid resized image")?,
            )
        }
        DynamicImage::ImageLumaA8(i) => {
            let from = i.dimensions();
            DynamicImage::ImageLumaA8(
                GrayAlphaImage::from_raw(
                    w,
                    h,
                    run(i.into_raw(), from, size, PixelType::U8x2, true)?,
                )
                .context("Invalid resized image")?,
            )
        }
        other => return lanczos(DynamicImage::ImageRgba8(other.into_rgba8()), size),
    })
}
/// Aspect-preserving size whose longer side is at most `cap` (never enlarges).
pub fn fit(dimensions: [u32; 2], cap: u32) -> [u32; 2] {
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

#[cfg(test)]
mod tests {
    use super::*;
    fn worst(a: &[u8], b: &[u8]) -> u8 {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap()
    }
    #[test]
    fn every_layout_matches_the_reference_lanczos_and_keeps_its_type() {
        let rgba = RgbaImage::from_fn(320, 200, |x, y| {
            image::Rgba([
                (128. + 100. * (x as f32 / 23.).sin()) as u8,
                (128. + 100. * (y as f32 / 17.).cos()) as u8,
                ((x + y) / 3) as u8,
                255,
            ])
        });
        let size = fit([320, 200], 96);
        assert_eq!(size, [96, 60]);
        for source in [
            DynamicImage::ImageRgba8(rgba.clone()),
            DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba.clone()).into_rgb8()),
            DynamicImage::ImageLuma8(DynamicImage::ImageRgba8(rgba.clone()).into_luma8()),
            DynamicImage::ImageRgb16(DynamicImage::ImageRgba8(rgba.clone()).into_rgb16()),
        ] {
            let color = source.color();
            let expected =
                source.resize_exact(size[0], size[1], image::imageops::FilterType::Lanczos3);
            let actual = lanczos(source, size).unwrap();
            assert_eq!((actual.width(), actual.height()), (96, 60));
            if color.bytes_per_pixel() == color.channel_count() {
                assert_eq!(actual.color(), color);
            }
            assert!(worst(actual.to_rgba8().as_raw(), expected.to_rgba8().as_raw()) <= 3);
        }
    }
    #[test]
    fn transparent_pixels_do_not_bleed_their_hidden_color() {
        // Left half opaque white, right half fully transparent black.
        let image = RgbaImage::from_fn(64, 64, |x, _| {
            if x < 32 {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let small = lanczos(DynamicImage::ImageRgba8(image), [16, 16])
            .unwrap()
            .into_rgba8();
        for p in small.pixels().filter(|p| p[3] > 16) {
            assert!(p[0] >= 235, "visible edge pixel darkened: {p:?}");
        }
        assert_eq!(fit([100, 50], 512), [100, 50]);
        assert_eq!(fit([4000, 1], 512), [512, 1]);
    }
}

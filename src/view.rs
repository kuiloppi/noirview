use crate::model::DisplayMode;
use eframe::egui::{Rect, Vec2};

/// The image occupies the full canvas. Fit leaves only aspect-ratio letterboxes;
/// Fill covers it and is clipped by the painter, without distorting the source.
pub fn image_rect(canvas: Rect, dimensions: [u32; 2], mode: DisplayMode) -> Rect {
    let natural = Vec2::new(dimensions[0].max(1) as f32, dimensions[1].max(1) as f32);
    let scale_x = canvas.width().max(1.) / natural.x;
    let scale_y = canvas.height().max(1.) / natural.y;
    let scale = match mode {
        DisplayMode::Fit => scale_x.min(scale_y),
        DisplayMode::Fill => scale_x.max(scale_y),
        DisplayMode::ShrinkOnly => scale_x.min(scale_y).min(1.),
    };
    Rect::from_center_size(canvas.center(), natural * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Pos2;
    #[test]
    fn fit_uses_every_available_pixel_for_a_matching_aspect_ratio() {
        let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920., 1080.));
        for dimensions in [[1920, 1080], [3840, 2160], [640, 360]] {
            assert_eq!(image_rect(canvas, dimensions, DisplayMode::Fit), canvas);
        }
    }
    #[test]
    fn fill_covers_all_edges_without_changing_aspect_ratio() {
        let canvas = Rect::from_min_size(Pos2::new(10., 20.), Vec2::new(1920., 1080.));
        for dimensions in [[800, 1200], [1200, 800], [3000, 1000], [400, 400]] {
            let rect = image_rect(canvas, dimensions, DisplayMode::Fill);
            assert!(rect.expand(0.001).contains_rect(canvas));
            assert!(rect.center().distance(canvas.center()) < 0.001);
            assert!(
                (rect.aspect_ratio() - dimensions[0] as f32 / dimensions[1] as f32).abs() < 0.0001
            );
            assert!(
                (rect.width() - canvas.width()).abs() < 0.001
                    || (rect.height() - canvas.height()).abs() < 0.001
            );
        }
    }
    #[test]
    fn fit_preserves_the_whole_image_and_shrink_only_preserves_small_sizes() {
        let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 800.));
        let fit = image_rect(canvas, [400, 600], DisplayMode::Fit);
        assert!(canvas.contains_rect(fit));
        assert_eq!(fit.height(), canvas.height());
        assert_eq!(
            image_rect(canvas, [400, 600], DisplayMode::ShrinkOnly).size(),
            Vec2::new(400., 600.)
        );
    }
}

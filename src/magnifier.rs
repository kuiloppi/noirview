use crate::model::Settings;
use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Stroke, StrokeKind, TextureId, Vec2};
use std::path::{Path, PathBuf};

/// A press belongs to the image on which it began. Hover is only an admission
/// check; losing hover during a drag must not release the magnifier.
#[derive(Default)]
pub struct Magnifier {
    owner: Option<PathBuf>,
    active: bool,
    /// A press admitted on this image, waiting for `delay` before magnifying.
    armed: Option<std::time::Instant>,
}
impl Magnifier {
    pub fn reset(&mut self) {
        self.active = false;
        self.armed = None;
        self.owner = None;
    }
    /// `delay` is zero in the ordinary viewer. The grid's big-image view uses a
    /// hold time so a short click can mean "back to the grid" instead.
    pub fn update(
        &mut self,
        path: &Path,
        ctx: &egui::Context,
        allowed: bool,
        enabled: bool,
        delay: std::time::Duration,
    ) -> bool {
        if self.owner.as_deref() != Some(path) {
            self.reset();
            self.owner = Some(path.into());
        }
        ctx.input(|i| {
            if !enabled || !i.focused || !i.pointer.primary_down() {
                self.active = false;
                self.armed = None;
            } else if i.pointer.primary_pressed() && allowed {
                self.armed = Some(std::time::Instant::now());
            }
        });
        if let Some(pressed) = self.armed {
            let held = pressed.elapsed();
            if held >= delay {
                self.active = true;
            } else {
                ctx.request_repaint_after(delay - held);
            }
        }
        self.active
    }
}

/// Keep the source point under the pointer fixed while magnifying. Moving the
/// pointer therefore pans the zoomed image without changing the saved view.
pub fn zoomed_rect(image: Rect, pointer: Pos2, zoom: f32) -> Rect {
    Rect::from_min_size(pointer - (pointer - image.min) * zoom, image.size() * zoom)
}

fn vertex(mesh: &mut Mesh, p: Pos2, image: Rect, alpha: u8) {
    mesh.vertices.push(egui::epaint::Vertex {
        pos: p,
        uv: Pos2::new(
            (p.x - image.left()) / image.width(),
            (p.y - image.top()) / image.height(),
        ),
        color: Color32::from_white_alpha(alpha),
    });
}

fn circular_mesh(texture: TextureId, center: Pos2, radius: f32, feather: f32, image: Rect) -> Mesh {
    let mut mesh = Mesh::with_texture(texture);
    vertex(&mut mesh, center, image, 255);
    let rings = if feather > 0. { 9 } else { 1 };
    for ring in 0..rings {
        let fraction = if rings == 1 {
            0.
        } else {
            ring as f32 / (rings - 1) as f32
        };
        let r = if rings == 1 {
            radius
        } else {
            radius - feather + feather * fraction
        };
        let alpha = ((1. - fraction) * 255.).round() as u8;
        for segment in 0..64 {
            let angle = std::f32::consts::TAU * segment as f32 / 64.;
            vertex(&mut mesh, center + Vec2::angled(angle) * r, image, alpha);
        }
    }
    for segment in 0..64_u32 {
        let next = (segment + 1) % 64;
        mesh.indices.extend_from_slice(&[0, 1 + segment, 1 + next]);
        for ring in 1..rings as u32 {
            let inner = 1 + (ring - 1) * 64;
            let outer = inner + 64;
            mesh.indices.extend_from_slice(&[
                inner + segment,
                outer + segment,
                outer + next,
                inner + segment,
                outer + next,
                inner + next,
            ]);
        }
    }
    mesh
}

fn window_mesh(texture: TextureId, area: Rect, feather: f32, image: Rect) -> Mesh {
    let mut mesh = Mesh::with_texture(texture);
    let inner = area.shrink(feather);
    for (rect, alpha) in [(inner, 255), (area, if feather > 0. { 0 } else { 255 })] {
        for p in [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ] {
            vertex(&mut mesh, p, image, alpha);
        }
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    for i in 0..4 {
        let next = (i + 1) % 4;
        mesh.indices
            .extend_from_slice(&[i, 4 + i, 4 + next, i, 4 + next, next]);
    }
    mesh
}

pub fn paint(
    painter: &egui::Painter,
    texture: TextureId,
    image: Rect,
    area: Rect,
    pointer: Pos2,
    settings: &Settings,
) {
    let pointer = image.clamp(pointer);
    let zoomed = zoomed_rect(image, pointer, settings.lens_zoom.clamp(1.1, 12.));
    let radius = settings.lens_px.clamp(80., 700.) / 2.;
    let feather = if settings.lens_soft_edges {
        settings.lens_feather_px.clamp(1., 60.)
    } else {
        0.
    };
    let mesh = if settings.lens_full_window {
        window_mesh(
            texture,
            area,
            feather.min(area.width().min(area.height()) / 3.),
            zoomed,
        )
    } else {
        circular_mesh(texture, pointer, radius, feather.min(radius * 0.8), zoomed)
    };
    let mut background = mesh.clone();
    background.texture_id = TextureId::default();
    for v in &mut background.vertices {
        v.uv = egui::epaint::WHITE_UV;
        v.color = Color32::from_rgba_unmultiplied(12, 15, 20, v.color.a());
    }
    // Same UI layer as the image: no foreground area can steal pointer hover or
    // cover toolbar/dialog controls. Scissoring also avoids stretched edge texels.
    let painter = painter.with_clip_rect(area);
    painter.add(egui::Shape::mesh(background));
    painter
        .with_clip_rect(area.intersect(zoomed))
        .add(egui::Shape::mesh(mesh));
    if settings.lens_border {
        let stroke = Stroke::new(2.0_f32, Color32::from_rgb(110, 224, 176));
        if settings.lens_full_window {
            painter.rect_stroke(area.shrink(1.), 0., stroke, StrokeKind::Inside);
        } else {
            painter.circle_stroke(pointer, radius, stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zoom_keeps_the_pointer_source_point_fixed_and_tracks_motion() {
        let image = Rect::from_min_size(Pos2::new(20., 40.), Vec2::new(800., 600.));
        for pointer in [image.min, image.center(), image.max] {
            let zoomed = zoomed_rect(image, pointer, 3.);
            assert_eq!(
                (pointer - image.min) / image.size(),
                (pointer - zoomed.min) / zoomed.size()
            );
            assert_eq!(zoomed.size(), image.size() * 3.);
        }
        assert_ne!(
            zoomed_rect(image, image.min, 3.).min,
            zoomed_rect(image, image.max, 3.).min
        );
    }
    #[test]
    fn feathered_meshes_have_opaque_detail_and_transparent_edges() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 600.));
        for mesh in [
            circular_mesh(TextureId::Managed(1), area.center(), 100., 20., area),
            window_mesh(TextureId::Managed(1), area, 20., area),
        ] {
            assert!(mesh.is_valid());
            assert_eq!(mesh.vertices.first().unwrap().color.a(), 255);
            assert_eq!(mesh.vertices.last().unwrap().color.a(), 0);
        }
    }
    #[test]
    fn a_hold_delay_postpones_magnifying_until_the_press_is_long_enough() {
        let ctx = egui::Context::default();
        let mut lens = Magnifier::default();
        let path = Path::new("a.jpg");
        let delay = std::time::Duration::from_millis(120);
        let mut frame = |events: Vec<egui::Event>| {
            let mut active = false;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| active = lens.update(path, ctx, true, true, delay),
            );
            active
        };
        let press = |pressed| egui::Event::PointerButton {
            pos: Pos2::new(10., 10.),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        assert!(!frame(vec![press(true)]));
        assert!(!frame(vec![]));
        std::thread::sleep(delay + std::time::Duration::from_millis(20));
        assert!(frame(vec![]));
        assert!(!frame(vec![press(false)]));
    }
    #[test]
    fn a_held_press_survives_hover_loss_and_movement_but_not_release_or_focus_loss() {
        let ctx = egui::Context::default();
        let mut lens = Magnifier::default();
        let path = Path::new("a.jpg");
        let mut frame = |events: Vec<egui::Event>, allowed, focused| {
            let mut active = false;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    focused,
                    ..Default::default()
                },
                |ctx| {
                    active = lens.update(path, ctx, allowed, true, std::time::Duration::ZERO);
                },
            );
            active
        };
        let press = |pressed| egui::Event::PointerButton {
            pos: Pos2::new(100., 100.),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(frame(vec![press(true)], true, true));
        for i in 0..30 {
            assert!(frame(
                vec![egui::Event::PointerMoved(Pos2::new(100. + i as f32, 130.))],
                false,
                true
            ));
        }
        assert!(!frame(vec![press(false)], false, true));
        assert!(!frame(vec![press(true)], false, true));
        assert!(!frame(vec![], true, true)); // dragging into the image does not start a new press
        assert!(!frame(vec![press(false)], true, true));
        assert!(frame(vec![press(true)], true, true));
        assert!(!frame(vec![], true, false));
        assert!(!frame(vec![], true, true));
    }
    #[test]
    fn changing_images_or_disabling_zoom_requires_a_new_press() {
        let ctx = egui::Context::default();
        let mut lens = Magnifier::default();
        let press = |pressed| egui::Event::PointerButton {
            pos: Pos2::new(100., 100.),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut frame = |path: &str, events, enabled| {
            let mut active = false;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    active = lens.update(
                        Path::new(path),
                        ctx,
                        true,
                        enabled,
                        std::time::Duration::ZERO,
                    );
                },
            );
            active
        };
        assert!(frame("a.jpg", vec![press(true)], true));
        assert!(!frame("b.jpg", vec![], true));
        assert!(!frame("b.jpg", vec![press(false)], true));
        assert!(frame("b.jpg", vec![press(true)], true));
        assert!(!frame("b.jpg", vec![], false));
        assert!(!frame("b.jpg", vec![], true));
    }
}

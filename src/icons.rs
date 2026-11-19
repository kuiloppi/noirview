use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
#[derive(Clone, Copy)]
pub enum Icon {
    Previous,
    Next,
    Tag,
    Trash,
    Heart,
    Filter,
    Slideshow,
    Grid,
    Music,
    Fullscreen,
    Move,
    Folder,
    Settings,
    Lock,
    Unlock,
    // Grid icon bar.
    Eye,
    Preview,
    Sort,
    Pages,
    SelectPage,
    SelectAll,
    ClearSelection,
    Restore,
    CloseGrid,
    Minus,
    Plus,
    FloatingTrash,
    AddPageSize,
    Order,
    Prompt,
    More,
    /// Restore deleted: a bin with an arrow up out of it.
    Undo,
    /// Fullscreen shows only the image: a picture in a frame.
    ImageOnly,
    /// Round notice: a circle closing on a check mark.
    Round,
    /// Go to image / page 1: an arrow to a bar.
    GoTo,
}
pub fn paint(p: &egui::Painter, center: Pos2, icon: Icon, color: Color32) {
    let s = Stroke::new(1.7_f32, color);
    let pt = |x: f32, y: f32| center + Vec2::new(x, y);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([pt(a.0, a.1), pt(b.0, b.1)], s);
    };
    let path = |v: &[(f32, f32)], closed: bool| {
        let points = v.iter().map(|(x, y)| pt(*x, *y)).collect();
        p.add(if closed {
            egui::Shape::closed_line(points, s)
        } else {
            egui::Shape::line(points, s)
        });
    };
    match icon {
        Icon::Previous => path(&[(3., -7.), (-4., 0.), (3., 7.)], false),
        Icon::Next => path(&[(-3., -7.), (4., 0.), (-3., 7.)], false),
        Icon::Tag => {
            path(
                &[(-8., -8.), (0., -8.), (9., 1.), (1., 9.), (-8., 0.)],
                true,
            );
            p.circle_stroke(pt(-3.5, -3.5), 1.5, s);
        }
        Icon::Round => {
            let points: Vec<_> = (0..=22)
                .map(|i| {
                    let a = std::f32::consts::PI * (-0.35 + 1.7 * i as f32 / 22.);
                    center + Vec2::angled(a) * 8.
                })
                .collect();
            p.add(egui::Shape::line(points, s));
            path(&[(-4., 0.), (-1., 3.), (4., -3.)], false);
        }
        Icon::GoTo => {
            line((-7., -7.), (-7., 7.));
            line((-4., 0.), (7., 0.));
            path(&[(-1., -4.), (-5., 0.), (-1., 4.)], false);
        }
        Icon::ImageOnly => {
            p.rect_stroke(
                Rect::from_center_size(center, Vec2::new(18., 14.)),
                2.,
                s,
                egui::StrokeKind::Inside,
            );
            path(
                &[(-6., 4.), (-2., -1.), (1., 2.), (3., 0.), (6., 4.)],
                false,
            );
            p.circle_filled(pt(3., -3.), 1.4, color);
        }
        Icon::Undo => {
            path(&[(-7., -1.), (-6., 9.), (6., 9.), (7., -1.)], false);
            line((0., 6.), (0., -9.));
            path(&[(-4., -5.), (0., -9.), (4., -5.)], false);
        }
        Icon::Trash => {
            line((-9., -6.), (9., -6.));
            path(&[(-6., -5.), (-5., 9.), (5., 9.), (6., -5.)], false);
            path(&[(-4., -6.), (-3., -10.), (3., -10.), (4., -6.)], false);
            line((-2., -2.), (-2., 5.));
            line((2., -2.), (2., 5.));
        }
        Icon::Heart => path(
            &[
                (0., 8.),
                (-8., 1.),
                (-9., -3.),
                (-7., -7.),
                (-3., -8.),
                (0., -5.),
                (3., -8.),
                (7., -7.),
                (9., -3.),
                (8., 1.),
            ],
            true,
        ),
        Icon::Filter => path(
            &[
                (-9., -8.),
                (9., -8.),
                (3., 0.),
                (3., 7.),
                (-3., 10.),
                (-3., 0.),
            ],
            true,
        ),
        Icon::Slideshow => {
            p.rect_stroke(
                Rect::from_center_size(center, Vec2::new(22., 17.)),
                3.,
                s,
                egui::StrokeKind::Inside,
            );
            path(&[(-2., -4.), (4., 0.), (-2., 4.)], true);
        }
        Icon::Grid => {
            for y in [-5., 5.] {
                for x in [-5., 5.] {
                    p.rect_stroke(
                        Rect::from_center_size(pt(x, y), Vec2::splat(7.)),
                        1.5,
                        s,
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        Icon::Music => {
            path(&[(-4., 5.), (-4., -6.), (7., -9.), (7., 3.)], false);
            p.circle_stroke(pt(-7., 5.), 3., s);
            p.circle_stroke(pt(4., 3.), 3., s);
            line((-4., -2.), (7., -5.));
        }
        Icon::Fullscreen => {
            for (x, y) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                path(
                    &[(x * 3., y * 9.), (x * 9., y * 9.), (x * 9., y * 3.)],
                    false,
                );
            }
        }
        Icon::Move => {
            line((-9., 0.), (8., 0.));
            path(&[(2., -6.), (8., 0.), (2., 6.)], false);
            path(&[(7., -9.), (11., -9.), (11., 9.), (7., 9.)], false);
        }
        Icon::Folder => path(
            &[
                (-10., 8.),
                (-10., -7.),
                (-3., -7.),
                (0., -4.),
                (10., -4.),
                (10., 8.),
            ],
            true,
        ),
        Icon::Lock | Icon::Unlock => {
            p.rect_stroke(
                egui::Rect::from_min_max(pt(-7., -1.), pt(7., 9.)),
                2.,
                s,
                egui::StrokeKind::Middle,
            );
            // The shackle: closed, or lifted and open on the right.
            let lift = if matches!(icon, Icon::Lock) { 0. } else { -3. };
            let right = if matches!(icon, Icon::Lock) {
                -1.
            } else {
                -5. + lift
            };
            path(
                &[
                    (-4.5, -1.),
                    (-4.5, -5. + lift),
                    (-2.5, -8. + lift),
                    (2.5, -8. + lift),
                    (4.5, -5. + lift),
                    (4.5, right),
                ],
                false,
            );
            p.circle_filled(pt(0., 4.), 1.4, color);
        }
        Icon::Eye => {
            path(
                &[(-10., 0.), (-5., -5.), (0., -6.5), (5., -5.), (10., 0.)],
                false,
            );
            path(
                &[(-10., 0.), (-5., 5.), (0., 6.5), (5., 5.), (10., 0.)],
                false,
            );
            p.circle_stroke(center, 2.8, s);
        }
        Icon::Preview => {
            p.rect_stroke(
                Rect::from_center_size(pt(-2., -2.), Vec2::new(16., 13.)),
                2.,
                s,
                egui::StrokeKind::Middle,
            );
            p.circle_filled(pt(5., 5.), 4.5, Color32::from_rgb(20, 24, 30));
            p.circle_stroke(pt(5., 5.), 3.5, s);
            line((7.5, 7.5), (10., 10.));
        }
        Icon::Sort => {
            line((-9., -6.), (6., -6.));
            line((-9., 0.), (2., 0.));
            line((-9., 6.), (-2., 6.));
        }
        Icon::Pages => {
            for (dx, dy) in [(-3., 3.), (0., 0.), (3., -3.)] {
                p.rect_filled(
                    Rect::from_center_size(pt(dx, dy), Vec2::new(11., 13.)),
                    1.5,
                    Color32::from_rgb(20, 24, 30),
                );
                p.rect_stroke(
                    Rect::from_center_size(pt(dx, dy), Vec2::new(11., 13.)),
                    1.5,
                    s,
                    egui::StrokeKind::Middle,
                );
            }
        }
        Icon::SelectPage | Icon::SelectAll | Icon::ClearSelection => {
            if matches!(icon, Icon::SelectAll) {
                path(&[(-6., -9.), (9., -9.), (9., 6.)], false);
            }
            p.rect_stroke(
                Rect::from_center_size(pt(-1., 1.), Vec2::splat(15.)),
                2.,
                s,
                egui::StrokeKind::Middle,
            );
            if matches!(icon, Icon::ClearSelection) {
                line((-5., -3.), (3., 5.));
                line((3., -3.), (-5., 5.));
            } else {
                path(&[(-5., 1.), (-2., 4.), (4., -3.)], false);
            }
        }
        Icon::Restore => {
            let points: Vec<_> = (0..=24)
                .map(|i| {
                    let a = std::f32::consts::PI * (0.25 + 1.5 * i as f32 / 24.);
                    center + Vec2::angled(a) * 8.
                })
                .collect();
            p.add(egui::Shape::line(points, s));
            let end = center + Vec2::angled(std::f32::consts::PI * 0.25) * 8.;
            p.line_segment([end, end + Vec2::new(-5., 0.)], s);
            p.line_segment([end, end + Vec2::new(0., 5.)], s);
        }
        Icon::CloseGrid => {
            for (x, y) in [(-5., -5.), (5., -5.), (-5., 5.)] {
                p.rect_stroke(
                    Rect::from_center_size(pt(x, y), Vec2::splat(7.)),
                    1.5,
                    s,
                    egui::StrokeKind::Inside,
                );
            }
            line((2., 2.), (8., 8.));
            line((8., 2.), (2., 8.));
        }
        Icon::Minus => line((-7., 0.), (7., 0.)),
        Icon::Plus => {
            line((-7., 0.), (7., 0.));
            line((0., -7.), (0., 7.));
        }
        Icon::AddPageSize => {
            // A bookmark with a plus: keep this page size.
            path(
                &[(-6., -9.), (6., -9.), (6., 9.), (0., 4.), (-6., 9.)],
                true,
            );
            line((-3., -3.), (3., -3.));
            line((0., -6.), (0., 0.));
        }
        Icon::More => {
            path(&[(-6., -6.), (0., 0.), (-6., 6.)], false);
            path(&[(1., -6.), (7., 0.), (1., 6.)], false);
        }
        Icon::Prompt => {
            // A speech bubble with text lines.
            path(
                &[
                    (-9., -8.),
                    (9., -8.),
                    (9., 4.),
                    (-2., 4.),
                    (-6., 9.),
                    (-6., 4.),
                    (-9., 4.),
                ],
                true,
            );
            line((-5., -4.), (5., -4.));
            line((-5., 0.), (2., 0.));
        }
        Icon::Order => {
            // Two crossing arrows (shuffle).
            path(&[(-9., -5.), (-4., -5.), (4., 5.), (9., 5.)], false);
            path(&[(-9., 5.), (-4., 5.), (4., -5.), (9., -5.)], false);
            path(&[(6., -8.), (9., -5.), (6., -2.)], false);
            path(&[(6., 2.), (9., 5.), (6., 8.)], false);
        }
        Icon::FloatingTrash => {
            p.circle_stroke(center, 10., s);
            line((-5., -3.), (5., -3.));
            path(&[(-3.5, -2.), (-3., 5.), (3., 5.), (3.5, -2.)], false);
            line((-1.5, -3.), (-1., -5.5));
            line((1.5, -3.), (1., -5.5));
        }
        Icon::Settings => {
            p.circle_stroke(center, 7., s);
            p.circle_stroke(center, 2.5, s);
            for i in 0..8 {
                let a = std::f32::consts::TAU * i as f32 / 8.;
                p.line_segment(
                    [
                        center + Vec2::angled(a) * 7.,
                        center + Vec2::angled(a) * 10.,
                    ],
                    s,
                );
            }
        }
    }
}

//! Pointer gestures with the left button held, and the short-click test used by
//! the grid's big-image view (a longer hold belongs to the magnifier).
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gesture {
    #[default]
    LeftToRight,
    RightToLeft,
    UpToDown,
    DownToUp,
}

impl Gesture {
    pub const ALL: [Gesture; 4] = [
        Gesture::LeftToRight,
        Gesture::RightToLeft,
        Gesture::UpToDown,
        Gesture::DownToUp,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Gesture::LeftToRight => "Left → right",
            Gesture::RightToLeft => "Right → left",
            Gesture::UpToDown => "Up → down",
            Gesture::DownToUp => "Down → up",
        }
    }
    /// One straight movement (`dx`, `dy` in points, y grows downward) of at
    /// least `min_px` in this direction, mostly along its axis, within `max_ms`.
    pub fn matches(self, dx: f32, dy: f32, held_ms: u128, min_px: f32, max_ms: u64) -> bool {
        if held_ms > max_ms as u128 {
            return false;
        }
        let (along, across) = match self {
            Gesture::LeftToRight => (dx, dy),
            Gesture::RightToLeft => (-dx, dy),
            Gesture::UpToDown => (dy, dx),
            Gesture::DownToUp => (-dy, dx),
        };
        along >= min_px.max(1.) && across.abs() <= along * 0.5
    }
}

/// A right-button drag in progress: the start time and the pointer path
/// (also drawn as the optional trail).
pub struct Track {
    pub started: std::time::Instant,
    pub points: Vec<eframe::egui::Pos2>,
}

impl Track {
    pub fn new(at: eframe::egui::Pos2) -> Self {
        Self {
            started: std::time::Instant::now(),
            points: vec![at],
        }
    }
    pub fn moved(&mut self, to: eframe::egui::Pos2) {
        if self.points.last().is_none_or(|p| p.distance(to) >= 2.) {
            self.points.push(to);
        }
    }
    /// Farther than a click from the start at any point.
    pub fn dragging(&self) -> bool {
        let start = self.points[0];
        self.points.iter().any(|p| p.distance(start) > CLICK_SLOP)
    }
    /// (dx, dy, milliseconds) from the press to `end`.
    pub fn finish(&self, end: eframe::egui::Pos2) -> (f32, f32, u128) {
        let d = end - self.points[0];
        (d.x, d.y, self.started.elapsed().as_millis())
    }
}

/// Pointer movement (points) still counted as a click rather than a drag.
pub const CLICK_SLOP: f32 = 8.;

/// A release counts as a click when it came before the hold time and the
/// pointer barely moved.
pub fn is_click(held_ms: u128, dx: f32, dy: f32, hold_ms: u64) -> bool {
    held_ms < hold_ms as u128 && dx.hypot(dy) <= CLICK_SLOP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_direction_needs_distance_straightness_and_time() {
        let g = Gesture::LeftToRight;
        assert!(g.matches(150., 20., 300, 120., 1000));
        assert!(!g.matches(100., 0., 300, 120., 1000), "too short");
        assert!(!g.matches(150., 90., 300, 120., 1000), "too diagonal");
        assert!(!g.matches(-150., 0., 300, 120., 1000), "wrong way");
        assert!(!g.matches(150., 0., 1500, 120., 1000), "too slow");
        assert!(Gesture::RightToLeft.matches(-200., -10., 200, 120., 1000));
        assert!(Gesture::UpToDown.matches(5., 130., 200, 120., 1000));
        assert!(Gesture::DownToUp.matches(-5., -130., 200, 120., 1000));
        assert!(!Gesture::DownToUp.matches(0., 130., 200, 120., 1000));
    }

    #[test]
    fn track_measures_from_the_press_and_detects_dragging() {
        use eframe::egui::Pos2;
        let mut t = Track::new(Pos2::new(10., 10.));
        t.moved(Pos2::new(11., 10.));
        t.moved(Pos2::new(14., 12.));
        assert!(!t.dragging());
        t.moved(Pos2::new(150., 20.));
        assert!(t.dragging());
        let (dx, dy, _) = t.finish(Pos2::new(160., 15.));
        assert_eq!((dx, dy), (150., 5.));
        assert!(Gesture::LeftToRight.matches(dx, dy, 300, 120., 2000));
    }

    #[test]
    fn click_is_short_and_still() {
        assert!(is_click(120, 2., 3., 250));
        assert!(!is_click(300, 0., 0., 250), "held: magnifier");
        assert!(!is_click(100, 20., 0., 250), "moved: drag");
    }
}

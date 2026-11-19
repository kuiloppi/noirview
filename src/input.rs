use std::time::{Duration, Instant};
#[derive(Default)]
pub struct Dwell {
    entered: Option<Instant>,
    fired: bool,
}
impl Dwell {
    pub fn update(&mut self, inside: bool, delay: Duration, now: Instant) -> bool {
        if !inside {
            self.entered = None;
            self.fired = false;
            return false;
        }
        let start = *self.entered.get_or_insert(now);
        if !self.fired && now.saturating_duration_since(start) >= delay {
            self.fired = true;
            true
        } else {
            false
        }
    }
    pub fn pending(&self) -> bool {
        self.entered.is_some() && !self.fired
    }
    pub fn timer(&self, delay: Duration, now: Instant) -> Option<Duration> {
        self.entered
            .filter(|_| !self.fired)
            .map(|start| delay.saturating_sub(now.saturating_duration_since(start)))
    }
    pub fn progress(&self, delay: Duration, now: Instant) -> f32 {
        self.entered
            .map(|t| {
                (now.saturating_duration_since(t).as_secs_f32() / delay.as_secs_f32().max(0.001))
                    .clamp(0., 1.)
            })
            .unwrap_or(0.)
    }
}
/// Converts raw wheel distance into ordered navigation steps without a time gate.
/// Consecutive steps are stored as runs so a coalesced burst does not allocate
/// one queue entry per notch. Only a bounded batch is applied per UI frame.
#[derive(Default)]
pub struct WheelNavigation {
    up_next: bool,
    remainder: f64,
    unit: Option<eframe::egui::MouseWheelUnit>,
    pending: std::collections::VecDeque<(bool, u64)>,
}
impl WheelNavigation {
    pub fn set_up_next(&mut self, up_next: bool) {
        if self.up_next != up_next {
            self.reset();
            self.up_next = up_next;
        }
    }
    pub fn reset(&mut self) {
        self.remainder = 0.;
        self.unit = None;
        self.pending.clear();
    }
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn steps(
        &mut self,
        events: &[eframe::egui::Event],
        active: bool,
        points_per_line: f32,
    ) -> Vec<bool> {
        use eframe::egui::{Event, MouseWheelUnit};
        if !active {
            self.reset();
            return Vec::new();
        }
        for event in events {
            match event {
                Event::WindowFocused(false) => self.reset(),
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                } => {
                    // egui treats Shift+wheel as horizontal scrolling.
                    if modifiers.shift || !delta.y.is_finite() || delta.y == 0. {
                        continue;
                    }
                    let distance = match unit {
                        MouseWheelUnit::Point => {
                            f64::from(delta.y) / f64::from(points_per_line.max(1.))
                        }
                        MouseWheelUnit::Line | MouseWheelUnit::Page => f64::from(delta.y),
                    };
                    if self.unit != Some(*unit)
                        || (self.remainder != 0. && self.remainder.signum() != distance.signum())
                    {
                        self.remainder = 0.;
                    }
                    self.unit = Some(*unit);
                    self.remainder += distance;
                    let whole = self.remainder.trunc();
                    self.remainder -= whole;
                    let count = whole.abs() as u64;
                    if count > 0 {
                        let next = (whole > 0.) == self.up_next;
                        if let Some((_, queued)) = self
                            .pending
                            .back_mut()
                            .filter(|(direction, _)| *direction == next)
                        {
                            *queued = queued.saturating_add(count);
                        } else {
                            self.pending.push_back((next, count));
                        }
                    }
                }
                _ => {}
            }
        }
        if self.pending.is_empty() {
            return Vec::new();
        }
        let mut steps = Vec::with_capacity(32);
        while steps.len() < 32 {
            let Some((next, count)) = self.pending.front_mut() else {
                break;
            };
            steps.push(*next);
            *count -= 1;
            if *count == 0 {
                self.pending.pop_front();
            }
        }
        steps
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_activation_per_entry() {
        let mut d = Dwell::default();
        let t = Instant::now();
        let delay = Duration::from_millis(100);
        assert!(!d.update(true, delay, t));
        assert!(d.update(true, delay, t + delay));
        assert!(!d.update(true, delay, t + delay * 10));
        assert!(!d.update(false, delay, t + delay * 11));
        assert!(!d.update(true, delay, t + delay * 12));
        assert!(d.update(true, delay, t + delay * 13));
    }
    #[test]
    fn dwell_schedules_first_frame_at_zero_progress_and_stops_after_firing_or_leaving() {
        let mut dwell = Dwell::default();
        let now = Instant::now();
        let delay = Duration::from_millis(100);
        assert_eq!(dwell.timer(delay, now), None);
        assert!(!dwell.update(true, delay, now));
        assert_eq!(dwell.progress(delay, now), 0.);
        assert!(dwell.pending());
        assert_eq!(dwell.timer(delay, now), Some(delay));
        assert_eq!(
            dwell.timer(delay, now + Duration::from_millis(25)),
            Some(Duration::from_millis(75))
        );
        assert!(dwell.update(true, delay, now + delay));
        assert!(!dwell.pending());
        assert_eq!(dwell.timer(delay, now + delay), None);
        dwell.update(false, delay, now + delay);
        assert_eq!(dwell.timer(delay, now + delay), None);
    }
    fn wheel(unit: eframe::egui::MouseWheelUnit, y: f32) -> eframe::egui::Event {
        eframe::egui::Event::MouseWheel {
            unit,
            delta: eframe::egui::vec2(0., y),
            modifiers: eframe::egui::Modifiers::NONE,
        }
    }
    #[test]
    fn rapid_discrete_notches_are_never_dropped_by_a_timer() {
        use eframe::egui::MouseWheelUnit::Line;
        let mut input = WheelNavigation::default();
        for _ in 0..20 {
            assert_eq!(input.steps(&[wheel(Line, -1.)], true, 40.), vec![true]);
        }
    }
    #[test]
    fn coalesced_events_preserve_every_step_and_direction_in_order() {
        use eframe::egui::MouseWheelUnit::Line;
        let mut input = WheelNavigation::default();
        assert_eq!(
            input.steps(
                &[wheel(Line, -3.), wheel(Line, 2.), wheel(Line, -1.)],
                true,
                40.
            ),
            vec![true, true, true, false, false, true]
        );
    }
    #[test]
    fn fractional_wheel_and_touchpad_distance_accumulate_across_frames() {
        use eframe::egui::MouseWheelUnit::{Line, Point};
        let mut input = WheelNavigation::default();
        for _ in 0..7 {
            assert!(input.steps(&[wheel(Line, -0.125)], true, 40.).is_empty());
        }
        assert_eq!(input.steps(&[wheel(Line, -0.125)], true, 40.), vec![true]);
        for _ in 0..3 {
            assert!(input.steps(&[wheel(Point, 10.)], true, 40.).is_empty());
        }
        assert_eq!(input.steps(&[wheel(Point, 10.)], true, 40.), vec![false]);
    }
    #[test]
    fn reversing_direction_or_changing_unit_does_not_inherit_partial_motion() {
        use eframe::egui::MouseWheelUnit::{Line, Point};
        let mut input = WheelNavigation::default();
        assert!(input.steps(&[wheel(Line, 0.75)], true, 40.).is_empty());
        assert_eq!(input.steps(&[wheel(Line, -1.)], true, 40.), vec![true]);
        assert!(input.steps(&[wheel(Point, 30.)], true, 40.).is_empty());
        assert!(input.steps(&[wheel(Line, 0.5)], true, 40.).is_empty());
        assert_eq!(input.steps(&[wheel(Line, 0.5)], true, 40.), vec![false]);
    }
    #[test]
    fn oversized_burst_is_drained_in_bounded_frames_without_losing_steps() {
        use eframe::egui::MouseWheelUnit::Line;
        let mut input = WheelNavigation::default();
        let mut result = input.steps(&[wheel(Line, -80.), wheel(Line, 2.)], true, 40.);
        assert_eq!(result.len(), 32);
        assert!(input.has_pending());
        while input.has_pending() {
            let batch = input.steps(&[], true, 40.);
            assert!(batch.len() <= 32);
            result.extend(batch);
        }
        assert_eq!(&result[..80], vec![true; 80]);
        assert_eq!(&result[80..], &[false, false]);
    }
    #[test]
    fn blocked_controls_and_focus_loss_clear_queued_and_partial_motion() {
        use eframe::egui::{Event, MouseWheelUnit::Line};
        let mut input = WheelNavigation::default();
        input.steps(&[wheel(Line, -80.)], true, 40.);
        assert!(input.steps(&[], false, 40.).is_empty());
        assert!(!input.has_pending());
        assert!(input.steps(&[], true, 40.).is_empty());
        input.steps(&[wheel(Line, 0.5)], true, 40.);
        assert!(input
            .steps(&[Event::WindowFocused(false), wheel(Line, 0.5)], true, 40.)
            .is_empty());
        assert_eq!(input.steps(&[wheel(Line, 0.5)], true, 40.), vec![false]);
    }
    #[test]
    fn horizontal_and_nonfinite_events_do_not_navigate() {
        use eframe::egui::{Event, Modifiers, MouseWheelUnit::Line};
        let mut input = WheelNavigation::default();
        let horizontal = Event::MouseWheel {
            unit: Line,
            delta: eframe::egui::vec2(1., 0.),
            modifiers: Modifiers::NONE,
        };
        let shifted = Event::MouseWheel {
            unit: Line,
            delta: eframe::egui::vec2(0., 1.),
            modifiers: Modifiers::SHIFT,
        };
        assert!(input
            .steps(
                &[
                    horizontal,
                    shifted,
                    wheel(Line, f32::NAN),
                    wheel(Line, f32::INFINITY)
                ],
                true,
                40.
            )
            .is_empty());
    }
    #[test]
    fn configured_scroll_up_advances_and_direction_changes_drop_old_queued_and_fractional_input() {
        use eframe::egui::MouseWheelUnit::{Line, Point};
        let mut input = WheelNavigation::default();
        input.set_up_next(true);
        assert_eq!(
            input.steps(&[wheel(Line, 3.), wheel(Line, -2.)], true, 40.),
            vec![true, true, true, false, false]
        );
        assert!(input.steps(&[wheel(Point, 20.)], true, 40.).is_empty());
        input.set_up_next(false);
        assert!(input.steps(&[wheel(Point, 20.)], true, 40.).is_empty());
        assert_eq!(input.steps(&[wheel(Point, 20.)], true, 40.), vec![false]);
        input.set_up_next(true);
        assert_eq!(input.steps(&[wheel(Line, 80.)], true, 40.), vec![true; 32]);
        assert!(input.has_pending());
        input.set_up_next(false);
        assert!(!input.has_pending());
        assert!(input.steps(&[], true, 40.).is_empty());
        assert_eq!(input.steps(&[wheel(Line, -1.)], true, 40.), vec![true]);
    }
}

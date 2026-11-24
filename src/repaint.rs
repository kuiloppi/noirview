//! Deadlines for application work; input, egui animation and workers wake separately.
use std::time::{Duration, Instant};
pub const ANIMATION: Duration = Duration::from_millis(16);
pub const SAVE_INTERVAL: Duration = Duration::from_millis(800);
pub const TOAST_LIFETIME: Duration = Duration::from_secs(5);
/// While images change (browsing), saves wait for a pause this long…
pub const BROWSE_PAUSE: Duration = Duration::from_millis(1500);
/// …or at most this long since the last save. On a hard disk, a save after
/// every image competes with reading the images.
pub const BROWSE_SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// Time until a pending save may run: `SAVE_INTERVAL` after the last save,
/// and while browsing not before a pause or `BROWSE_SAVE_INTERVAL`.
pub fn save_due(last_save: Instant, image_changed: Instant, now: Instant) -> Duration {
    let normal = remaining(last_save, SAVE_INTERVAL, now);
    let paused = remaining(image_changed, BROWSE_PAUSE, now);
    let longest = remaining(last_save, BROWSE_SAVE_INTERVAL, now);
    normal.max(paused.min(longest))
}
pub fn remaining(start: Instant, interval: Duration, now: Instant) -> Duration {
    interval.saturating_sub(now.saturating_duration_since(start))
}
pub fn slide_interval(seconds: f32) -> Duration {
    Duration::from_secs_f32(if seconds.is_finite() {
        seconds.clamp(0.1, 86400.)
    } else {
        4.
    })
}
pub fn next_minute(now: chrono::DateTime<chrono::Local>) -> Duration {
    use chrono::Timelike;
    Duration::from_secs(60 - now.second() as u64)
        .saturating_sub(Duration::from_nanos(now.nanosecond() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_toast_and_slideshow_deadlines_keep_their_remaining_time() {
        let start = Instant::now();
        assert_eq!(
            remaining(start, SAVE_INTERVAL, start + Duration::from_millis(300)),
            Duration::from_millis(500)
        );
        assert_eq!(
            remaining(start, TOAST_LIFETIME, start + Duration::from_secs(2)),
            Duration::from_secs(3)
        );
        assert_eq!(
            remaining(start, slide_interval(4.), start + Duration::from_secs(3)),
            Duration::from_secs(1)
        );
        assert_eq!(
            remaining(start, SAVE_INTERVAL, start + Duration::from_secs(2)),
            Duration::ZERO
        );
        assert_eq!(slide_interval(f32::NAN), Duration::from_secs(4));
    }
    #[test]
    fn saves_wait_for_a_pause_in_browsing_or_five_seconds() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        // Not browsing: the usual interval.
        let long_ago = start;
        assert_eq!(save_due(start, long_ago, at(2000)), Duration::ZERO);
        assert_eq!(
            save_due(at(1500), long_ago, at(2000)),
            Duration::from_millis(300)
        );
        // Browsing (an image 200 ms ago): until the pause.
        assert_eq!(
            save_due(start, at(1800), at(2000)),
            Duration::from_millis(1300)
        );
        // Still browsing but five seconds since the last save: now.
        assert_eq!(save_due(start, at(4900), at(5000)), Duration::ZERO);
        assert_eq!(
            save_due(start, at(4500), at(4600)),
            Duration::from_millis(400)
        );
    }
    #[test]
    fn scheduled_backups_recheck_at_the_next_wall_clock_minute() {
        use chrono::{TimeZone, Timelike};
        let time = chrono::Local
            .timestamp_opt(1800000000, 750_000_000)
            .unwrap()
            .with_second(58)
            .unwrap();
        assert_eq!(next_minute(time), Duration::from_millis(1250));
        assert_eq!(
            next_minute(time.with_second(0).unwrap().with_nanosecond(0).unwrap()),
            Duration::from_secs(60)
        );
    }
}

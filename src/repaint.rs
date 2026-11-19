//! Deadlines for application work; input, egui animation and workers wake separately.
use std::time::{Duration, Instant};
pub const ANIMATION: Duration = Duration::from_millis(16);
pub const SAVE_INTERVAL: Duration = Duration::from_millis(800);
pub const TOAST_LIFETIME: Duration = Duration::from_secs(5);
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

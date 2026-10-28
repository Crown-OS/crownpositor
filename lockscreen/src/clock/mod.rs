mod morph;

use std::{fmt::Write, time::Duration};

use chrono::{
    DateTime, Local, Timelike,
    format::{Item, StrftimeItems},
};
use crownos_config::schema::Lock;

pub use crate::util::homography::Homography;
pub use morph::{MORPH_BLUR, MORPH_DURATION, morph};

/// How white glass digits wash at their top and at their baseline.
pub const GLASS_TINT: [f32; 2] = [0.28, 0.9];

const DEFAULT_TIME: &str = "%H:%M";
const DEFAULT_DATE: &str = "%A, %e %B";

/// The clock and date as they read right now.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct ClockFace {
    pub time: String,
    pub date: String,
}

impl ClockFace {
    pub fn at(config: &Lock, now: DateTime<Local>) -> Self {
        Self {
            time: format(now, &config.time_format, DEFAULT_TIME),
            date: format(now, &config.date_format, DEFAULT_DATE)
                .trim()
                .to_owned(),
        }
    }

    pub fn now(config: &Lock) -> Self {
        Self::at(config, Local::now())
    }

    /// The hours, and everything after the first separator: the two groups
    /// the clock places on their own.
    pub fn groups(&self) -> (&str, &str) {
        self.time.split_once(':').unwrap_or((&self.time, ""))
    }
}

/// How long from now until the face reads differently.
pub fn next_change_in(config: &Lock) -> Duration {
    until_next_change(config, Local::now())
}

/// How long after `now` until the face reads differently.
pub fn until_next_change(config: &Lock, now: DateTime<Local>) -> Duration {
    let shows_seconds = [&config.time_format, &config.date_format]
        .iter()
        .any(|format| {
            ["%S", "%T", "%r", "%s", "%X", "%c"]
                .iter()
                .any(|spec| format.contains(spec))
        });
    let nanos_into_second = u64::from(now.nanosecond() % 1_000_000_000);
    let to_next_second = Duration::from_nanos(1_000_000_000 - nanos_into_second);
    if shows_seconds {
        to_next_second
    } else {
        Duration::from_secs(u64::from(59 - now.second().min(59))) + to_next_second
    }
}

/// Formats with `pattern`, or with `fallback` when `pattern` does not parse:
/// chrono panics on a bad pattern rather than erroring.
fn format(now: DateTime<Local>, pattern: &str, fallback: &str) -> String {
    let items = parse(pattern)
        .or_else(|| parse(fallback))
        .unwrap_or_default();
    let mut text = String::new();
    if write!(text, "{}", now.format_with_items(items.iter())).is_err() {
        text.clear();
    }
    text
}

fn parse(pattern: &str) -> Option<Vec<Item<'_>>> {
    let items: Vec<_> = StrftimeItems::new(pattern).collect();
    (!items.contains(&Item::Error)).then_some(items)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn noon() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 16, 12, 34, 56)
            .single()
            .expect("unambiguous")
    }

    #[test]
    fn a_bad_pattern_falls_back_instead_of_panicking() {
        let config = Lock {
            time_format: "%Q".into(),
            ..Lock::default()
        };
        assert_eq!(ClockFace::at(&config, noon()).time, "12:34");
    }

    #[test]
    fn the_time_splits_into_hours_and_the_rest() {
        let seconds = Lock {
            time_format: "%H:%M:%S".into(),
            ..Lock::default()
        };
        let face = ClockFace::at(&seconds, noon());
        assert_eq!(face.groups(), ("12", "34:56"));
        assert_eq!(face.date, "Friday, 16 October");
        let bare = ClockFace {
            time: "1234".into(),
            date: String::new(),
        };
        assert_eq!(bare.groups(), ("1234", ""));
    }

    #[test]
    fn minute_clocks_wake_on_the_minute() {
        let wait = until_next_change(&Lock::default(), noon());
        assert_eq!(wait.as_secs(), 4);
        let seconds = Lock {
            time_format: "%H:%M:%S".into(),
            ..Lock::default()
        };
        assert!(until_next_change(&seconds, noon()) <= Duration::from_secs(1));
    }
}

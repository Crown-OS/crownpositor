use std::time::{Duration, Instant};

/// A value that runs between 0 and 1 at constant speed and can turn back
/// mid-way without jumping.
#[derive(Debug, Clone, Copy)]
pub(super) struct Tween {
    from: f32,
    rising: bool,
    since: Instant,
}

impl Tween {
    pub fn at_rest(now: Instant) -> Self {
        Self {
            from: 0.0,
            rising: false,
            since: now,
        }
    }

    pub fn value(&self, now: Instant, duration: Duration) -> f32 {
        let travelled = if duration.is_zero() {
            1.0
        } else {
            now.saturating_duration_since(self.since).as_secs_f32() / duration.as_secs_f32()
        };
        let value = if self.rising {
            self.from + travelled
        } else {
            self.from - travelled
        };
        value.clamp(0.0, 1.0)
    }

    pub fn rising(&self) -> bool {
        self.rising
    }

    pub fn head(&mut self, rising: bool, now: Instant, duration: Duration) {
        if rising != self.rising {
            self.from = self.value(now, duration);
            self.rising = rising;
            self.since = now;
        }
    }

    pub fn is_settled(&self, now: Instant, duration: Duration) -> bool {
        let value = self.value(now, duration);
        if self.rising {
            value >= 1.0
        } else {
            value <= 0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_back_starts_from_where_it_was() {
        let start = Instant::now();
        let duration = Duration::from_millis(100);
        let mut tween = Tween::at_rest(start);
        tween.head(true, start, duration);
        let midway = start + Duration::from_millis(60);
        assert!((tween.value(midway, duration) - 0.6).abs() < 1e-3);
        tween.head(false, midway, duration);
        let later = midway + Duration::from_millis(20);
        assert!((tween.value(later, duration) - 0.4).abs() < 1e-3);
        assert!(tween.is_settled(later + duration, duration));
    }
}

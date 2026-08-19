//! Release velocity for direct-manipulation gestures.
//!
//! A gesture that hands off to a spring needs the speed the input was moving at
//! the instant it ended, not the average over the whole drag. This keeps a short
//! sliding window of positions and differentiates across it, so a slow drag
//! followed by a flick reads as a flick.
//!
//! Units are whatever the caller pushes in, per second.

use std::{collections::VecDeque, time::Duration};

/// Motion older than this is dropped before differentiating.
const WINDOW: Duration = Duration::from_millis(150);
/// Fraction of velocity retained per millisecond of stillness before the
/// release. Fingers that come to a stop and then lift must not fling.
const DECAY_PER_MS: f64 = 0.997;

#[derive(Debug, Clone, Copy)]
struct Sample {
    position: (f64, f64),
    at: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct VelocityTracker {
    samples: VecDeque<Sample>,
    position: (f64, f64),
}

impl VelocityTracker {
    /// Resets to the origin and anchors the window at `at`.
    ///
    /// The anchor matters: without a sample from before the first motion, a
    /// two-event flick has no time span to differentiate over and reports zero.
    pub fn begin(&mut self, at: Duration) {
        self.samples.clear();
        self.position = (0.0, 0.0);
        self.samples.push_back(Sample {
            position: self.position,
            at,
        });
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.position = (0.0, 0.0);
    }

    /// Cumulative travel since [`begin`](Self::begin).
    pub fn position(&self) -> (f64, f64) {
        self.position
    }

    pub fn push(&mut self, delta: (f64, f64), at: Duration) {
        self.position.0 += delta.0;
        self.position.1 += delta.1;
        self.samples.push_back(Sample {
            position: self.position,
            at,
        });
        self.trim(at);
    }

    /// Units per second at `at`, decayed by however long the input has been
    /// still.
    pub fn velocity(&self, at: Duration) -> (f64, f64) {
        let (Some(first), Some(last)) = (self.samples.front(), self.samples.back()) else {
            return (0.0, 0.0);
        };
        let span = last.at.saturating_sub(first.at).as_secs_f64();
        if span <= 0.0 {
            return (0.0, 0.0);
        }

        let idle = at.saturating_sub(last.at).as_secs_f64() * 1000.0;
        let decay = DECAY_PER_MS.powf(idle);

        (
            (last.position.0 - first.position.0) / span * decay,
            (last.position.1 - first.position.1) / span * decay,
        )
    }

    /// Always leaves two samples behind, so a gesture that pauses and then
    /// lifts still has a span to differentiate over rather than reporting a
    /// hard zero.
    fn trim(&mut self, now: Duration) {
        while self.samples.len() > 2
            && self
                .samples
                .front()
                .is_some_and(|sample| now.saturating_sub(sample.at) > WINDOW)
        {
            self.samples.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_steady_drag_reports_its_speed() {
        let mut tracker = VelocityTracker::default();
        tracker.begin(ms(0));
        for frame in 1..=10 {
            tracker.push((10.0, 0.0), ms(frame * 10));
        }
        // 10 px every 10 ms is 1000 px/s.
        let (x, y) = tracker.velocity(ms(100));
        assert!((x - 1000.0).abs() < 1.0, "got {x}");
        assert_eq!(y, 0.0);
    }

    #[test]
    fn a_flick_at_the_end_outweighs_the_crawl_before_it() {
        let mut tracker = VelocityTracker::default();
        tracker.begin(ms(0));

        // 400 ms of crawling...
        for frame in 1..=40 {
            tracker.push((1.0, 0.0), ms(frame * 10));
        }
        let crawling = tracker.velocity(ms(400)).0;
        assert!((crawling - 100.0).abs() < 1.0, "got {crawling}");

        // ...then 50 ms of sprinting. The window still holds part of the crawl,
        // so this is not the sprint's own speed — but it is unmistakably a
        // flick, which is what the release has to decide on.
        for frame in 1..=5 {
            tracker.push((20.0, 0.0), ms(400 + frame * 10));
        }
        let flicked = tracker.velocity(ms(450)).0;
        assert!(flicked > crawling * 5.0, "{flicked} against {crawling}");
    }

    #[test]
    fn motion_older_than_the_window_is_forgotten_entirely() {
        let mut tracker = VelocityTracker::default();
        tracker.begin(ms(0));
        for frame in 1..=100 {
            tracker.push((1.0, 0.0), ms(frame * 10));
        }
        // A whole second of travel, but only the last 150 ms is measured.
        let (x, _) = tracker.velocity(ms(1000));
        assert!((x - 100.0).abs() < 1.0, "got {x}");
    }

    #[test]
    fn a_single_event_still_has_a_span_to_measure() {
        let mut tracker = VelocityTracker::default();
        tracker.begin(ms(0));
        tracker.push((16.0, 0.0), ms(16));
        let (x, _) = tracker.velocity(ms(16));
        assert!((x - 1000.0).abs() < 1.0, "got {x}");
    }

    #[test]
    fn stillness_before_the_release_bleeds_the_fling_away() {
        let mut tracker = VelocityTracker::default();
        tracker.begin(ms(0));
        for frame in 1..=10 {
            tracker.push((10.0, 0.0), ms(frame * 10));
        }

        let moving = tracker.velocity(ms(100)).0;
        let rested = tracker.velocity(ms(600)).0;
        assert!(rested < moving * 0.5, "{rested} is not much slower than {moving}");
    }

    #[test]
    fn an_untouched_tracker_is_not_moving() {
        assert_eq!(VelocityTracker::default().velocity(ms(10)), (0.0, 0.0));

        let mut tracker = VelocityTracker::default();
        tracker.begin(ms(5));
        assert_eq!(tracker.velocity(ms(5)), (0.0, 0.0));
    }

    #[test]
    fn both_axes_are_tracked_independently() {
        let mut tracker = VelocityTracker::default();
        tracker.begin(ms(0));
        tracker.push((10.0, -20.0), ms(10));
        assert_eq!(tracker.position(), (10.0, -20.0));

        let (x, y) = tracker.velocity(ms(10));
        assert!((x - 1000.0).abs() < 1.0);
        assert!((y + 2000.0).abs() < 1.0);
    }
}

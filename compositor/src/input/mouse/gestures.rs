//! Touchpad swipe recognition.
//!
//! Gestures resolve to an [`Action`](crate::input::shortcuts::action::Action)
//! through `GestureBindings`, down the same path as keyboard chords.

use std::{collections::VecDeque, time::Duration};

/// Motion older than this is dropped before computing the release velocity, so
/// a slow drag followed by a flick reads as a flick.
const HISTORY_LIMIT: Duration = Duration::from_millis(150);
/// Fraction of velocity retained per millisecond after the fingers lift.
const DECELERATION_TOUCHPAD: f64 = 0.997;
/// Logical pixels of travel before a swipe commits to an axis.
const AXIS_LOCK_THRESHOLD: f64 = 12.0;
/// Travel (or fling velocity) needed for a swipe to fire rather than snap back.
const COMMIT_DISTANCE: f64 = 96.0;
const COMMIT_VELOCITY: f64 = 0.5;

#[derive(Debug, Clone, Copy)]
pub struct SwipeEvent {
    delta: (f64, f64),
    timestamp: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fingers {
    Two,
    Three,
    Four,
    Five,
}

impl Fingers {
    pub fn from_count(count: u32) -> Option<Self> {
        match count {
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            4 => Some(Self::Four),
            5 => Some(Self::Five),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SwipeGesture {
    LeftToRight(Fingers),
    RightToLeft(Fingers),
    BottomToTop(Fingers),
    TopToBottom(Fingers),
}

/// Which axis a swipe locked onto once it passed [`AXIS_LOCK_THRESHOLD`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// Progress of an in-flight swipe, so the workspace switch can follow the
/// fingers instead of jumping on release.
#[derive(Debug, Clone, Copy)]
pub struct GestureUpdate {
    /// Travel along the locked axis, in logical pixels.
    pub delta: f64,
    /// `-1.0..=1.0`, saturating at [`COMMIT_DISTANCE`].
    pub progress: f64,
}

#[derive(Debug, Clone, Default)]
pub struct GestureState {
    fingers: Option<Fingers>,
    axis: Option<Axis>,
    delta: (f64, f64),
    history: VecDeque<SwipeEvent>,
}

impl GestureState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.fingers.is_some()
    }

    pub fn fingers(&self) -> Option<Fingers> {
        self.fingers
    }

    /// An unrecognised finger count leaves the state inactive, so later updates
    /// are ignored rather than accumulating into a phantom gesture.
    pub fn begin(&mut self, count: u32) {
        *self = Self {
            fingers: Fingers::from_count(count),
            ..Self::default()
        };
    }

    pub fn update(&mut self, delta: (f64, f64), timestamp: Duration) -> Option<GestureUpdate> {
        self.fingers?;

        self.delta.0 += delta.0;
        self.delta.1 += delta.1;
        self.history.push_back(SwipeEvent { delta, timestamp });
        self.trim_history(timestamp);

        if self.axis.is_none() {
            let (x, y) = (self.delta.0.abs(), self.delta.1.abs());
            if x.max(y) >= AXIS_LOCK_THRESHOLD {
                self.axis = Some(if x >= y { Axis::Horizontal } else { Axis::Vertical });
            }
        }

        let travelled = match self.axis? {
            Axis::Horizontal => self.delta.0,
            Axis::Vertical => self.delta.1,
        };

        Some(GestureUpdate {
            delta: travelled,
            progress: (travelled / COMMIT_DISTANCE).clamp(-1.0, 1.0),
        })
    }

    /// Returns the gesture that fired, or `None` if the swipe was cancelled or
    /// travelled too little and too slowly to commit.
    pub fn end(&mut self, cancelled: bool, timestamp: Duration) -> Option<SwipeGesture> {
        let fingers = self.fingers.take()?;
        let axis = self.axis.take();
        let delta = std::mem::take(&mut self.delta);
        let velocity = self.velocity(timestamp);
        self.history.clear();

        if cancelled {
            return None;
        }

        let (travelled, speed) = match axis? {
            Axis::Horizontal => (delta.0, velocity.0),
            Axis::Vertical => (delta.1, velocity.1),
        };

        // A short but fast flick counts, so quick swipes don't need a full drag.
        let committed = travelled.abs() >= COMMIT_DISTANCE || speed.abs() >= COMMIT_VELOCITY;
        if !committed {
            return None;
        }

        Some(match axis? {
            Axis::Horizontal if travelled > 0.0 => SwipeGesture::LeftToRight(fingers),
            Axis::Horizontal => SwipeGesture::RightToLeft(fingers),
            Axis::Vertical if travelled > 0.0 => SwipeGesture::TopToBottom(fingers),
            Axis::Vertical => SwipeGesture::BottomToTop(fingers),
        })
    }

    fn trim_history(&mut self, now: Duration) {
        while let Some(front) = self.history.front() {
            if now.saturating_sub(front.timestamp) > HISTORY_LIMIT {
                self.history.pop_front();
            } else {
                break;
            }
        }
    }

    /// Logical pixels per millisecond, decayed over the gap between the last
    /// motion and the release.
    fn velocity(&self, now: Duration) -> (f64, f64) {
        let (Some(front), Some(back)) = (self.history.front(), self.history.back()) else {
            return (0.0, 0.0);
        };
        let span = back.timestamp.saturating_sub(front.timestamp).as_secs_f64() * 1000.0;
        if span <= 0.0 {
            return (0.0, 0.0);
        }

        let sum = self
            .history
            .iter()
            .fold((0.0, 0.0), |acc, e| (acc.0 + e.delta.0, acc.1 + e.delta.1));

        let idle = now.saturating_sub(back.timestamp).as_secs_f64() * 1000.0;
        let decay = DECELERATION_TOUCHPAD.powf(idle);

        (sum.0 / span * decay, sum.1 / span * decay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn unrecognised_finger_counts_stay_inactive() {
        let mut state = GestureState::new();
        state.begin(1);
        assert!(!state.is_active());
        assert!(state.update((100.0, 0.0), ms(10)).is_none());
    }

    #[test]
    fn axis_locks_only_after_the_threshold() {
        let mut state = GestureState::new();
        state.begin(3);
        assert!(
            state.update((4.0, 0.0), ms(10)).is_none(),
            "below the threshold there is no axis yet"
        );
        assert!(state.update((20.0, 3.0), ms(20)).is_some());
    }

    #[test]
    fn a_long_drag_commits() {
        let mut state = GestureState::new();
        state.begin(3);
        state.update((60.0, 0.0), ms(10));
        state.update((60.0, 0.0), ms(20));
        assert_eq!(
            state.end(false, ms(30)),
            Some(SwipeGesture::LeftToRight(Fingers::Three))
        );
    }

    #[test]
    fn a_short_slow_drag_snaps_back() {
        let mut state = GestureState::new();
        state.begin(3);
        state.update((20.0, 0.0), ms(100));
        state.update((5.0, 0.0), ms(400));
        assert_eq!(state.end(false, ms(900)), None);
    }

    #[test]
    fn cancelling_never_fires() {
        let mut state = GestureState::new();
        state.begin(4);
        state.update((0.0, -200.0), ms(10));
        assert_eq!(state.end(true, ms(20)), None);
        assert!(!state.is_active(), "a cancelled gesture is fully reset");
    }

    #[test]
    fn vertical_direction_matches_travel_sign() {
        let mut state = GestureState::new();
        state.begin(4);
        state.update((0.0, -120.0), ms(10));
        assert_eq!(
            state.end(false, ms(20)),
            Some(SwipeGesture::BottomToTop(Fingers::Four)),
            "negative y is upward travel"
        );
    }
}

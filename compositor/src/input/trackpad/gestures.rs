use std::time::Duration;

use spacecontrol::animations::velocity::VelocityTracker;

use crate::input::trackpad::catch_up::SWIPE_DEAD_ZONE;

/// Unaccelerated swipe units per millimetre of finger travel, on every touchpad.
///
/// libinput normalizes each pad to 1000 dpi and then scales its "unaccelerated"
/// deltas by the touchpad constant filter (0.9 × `TP_MAGIC_SLOWDOWN` 0.2968).
pub const UNITS_PER_MM: f64 = 1000.0 / 25.4 * 0.9 * 0.2968;

/// Travel before a swipe commits to an axis. libinput has already sat out
/// 1.5–2.5 mm before reporting the swipe at all, so this only has to pick a
/// direction, not reject noise.
const AXIS_LOCK_THRESHOLD: f64 = 0.5 * UNITS_PER_MM;
/// Travel needed for a swipe to fire rather than snap back.
const COMMIT_DISTANCE: f64 = 12.0 * UNITS_PER_MM;
/// Release speed that fires a swipe on its own: an unhurried flick.
const COMMIT_VELOCITY: f64 = 80.0 * UNITS_PER_MM;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fingers {
    Three,
    Four,
    Five,
}

impl Fingers {
    pub fn from_count(count: u32) -> Option<Self> {
        match count {
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
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Progress of an in-flight swipe, so an interactive consumer can follow the
/// fingers instead of waiting for the release.
#[derive(Debug, Clone, Copy)]
pub struct GestureUpdate {
    pub fingers: Fingers,
    pub axis: Axis,
    /// Cumulative travel along the locked axis, in unaccelerated units,
    /// including the travel libinput spent recognising the swipe.
    pub delta: f64,
}

/// What lifting the fingers produced.
#[derive(Debug, Clone, Copy)]
pub struct SwipeRelease {
    /// Unaccelerated units per second along the locked axis at the moment of
    /// release, signed like the travel. An interactive consumer projects this
    /// forward to pick where its spring should settle.
    pub velocity: f64,
    /// The touchpad withdrew the gesture — a palm, or a fifth finger landing.
    pub cancelled: bool,
    /// `None` when the swipe was cancelled, never locked an axis, or travelled
    /// too little and too slowly to commit.
    pub gesture: Option<SwipeGesture>,
}

#[derive(Debug, Clone, Default)]
pub struct GestureState {
    fingers: Option<Fingers>,
    axis: Option<Axis>,
    /// [`SWIPE_DEAD_ZONE`], signed the way the swipe locked.
    head_start: f64,
    motion: VelocityTracker,
}

impl GestureState {
    pub fn new() -> Self {
        Self::default()
    }

    /// An unrecognised finger count leaves the state inactive, so later updates
    /// are ignored rather than accumulating into a phantom gesture.
    pub fn begin(&mut self, count: u32, at: Duration) {
        self.fingers = Fingers::from_count(count);
        self.axis = None;
        self.head_start = 0.0;
        self.motion.begin(at);
    }

    pub fn update(&mut self, delta: (f64, f64), at: Duration) -> Option<GestureUpdate> {
        let fingers = self.fingers?;
        self.motion.push(delta, at);

        let (x, y) = self.motion.position();
        if self.axis.is_none() && x.abs().max(y.abs()) >= AXIS_LOCK_THRESHOLD {
            let (axis, travel) = if x.abs() >= y.abs() {
                (Axis::Horizontal, x)
            } else {
                (Axis::Vertical, y)
            };
            self.axis = Some(axis);
            self.head_start = SWIPE_DEAD_ZONE.copysign(travel);
        }

        let axis = self.axis?;
        Some(GestureUpdate {
            fingers,
            axis,
            delta: along(axis, (x, y)) + self.head_start,
        })
    }

    /// `None` only when no gesture was in progress.
    pub fn end(&mut self, cancelled: bool, at: Duration) -> Option<SwipeRelease> {
        let fingers = self.fingers.take()?;
        let axis = self.axis.take();
        let (x, y) = self.motion.position();
        let (vx, vy) = self.motion.velocity(at);
        self.motion.clear();

        // The head start is travel, not speed: it moves the release point but
        // must not make a slow swipe read as a flick.
        let (travelled, velocity) = match axis {
            Some(axis) => (along(axis, (x, y)) + self.head_start, along(axis, (vx, vy))),
            None => (0.0, 0.0),
        };

        // A short but fast flick counts, so quick swipes don't need a full drag.
        let committed =
            !cancelled && (travelled.abs() >= COMMIT_DISTANCE || velocity.abs() >= COMMIT_VELOCITY);

        let gesture = axis
            .filter(|_| committed)
            .map(|axis| match (axis, travelled > 0.0) {
                (Axis::Horizontal, true) => SwipeGesture::LeftToRight(fingers),
                (Axis::Horizontal, false) => SwipeGesture::RightToLeft(fingers),
                (Axis::Vertical, true) => SwipeGesture::TopToBottom(fingers),
                (Axis::Vertical, false) => SwipeGesture::BottomToTop(fingers),
            });

        Some(SwipeRelease {
            velocity,
            cancelled,
            gesture,
        })
    }
}

fn along(axis: Axis, (x, y): (f64, f64)) -> f64 {
    match axis {
        Axis::Horizontal => x,
        Axis::Vertical => y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn started(count: u32) -> GestureState {
        let mut state = GestureState::new();
        state.begin(count, ms(0));
        state
    }

    #[test]
    fn unrecognised_finger_counts_stay_inactive() {
        let mut state = started(1);
        assert!(state.update((100.0, 0.0), ms(10)).is_none());
        assert!(state.end(false, ms(20)).is_none());
    }

    #[test]
    fn axis_locks_only_after_the_threshold() {
        let mut state = started(3);
        assert!(
            state
                .update((AXIS_LOCK_THRESHOLD * 0.3, 0.0), ms(10))
                .is_none(),
            "below the threshold there is no axis yet"
        );
        let update = state
            .update((AXIS_LOCK_THRESHOLD, AXIS_LOCK_THRESHOLD * 0.2), ms(20))
            .expect("locked by now");
        assert_eq!(update.axis, Axis::Horizontal);
        assert_eq!(update.fingers, Fingers::Three);
    }

    #[test]
    fn updates_report_cumulative_travel_along_the_locked_axis() {
        let mut state = started(4);
        state.update((100.0, 0.0), ms(10));
        let update = state.update((50.0, 0.0), ms(20)).expect("active");
        assert_eq!(update.delta, 150.0 + SWIPE_DEAD_ZONE);

        // Off-axis motion is dropped once the axis has locked, so a swipe that
        // drifts diagonally still tracks in a straight line.
        let update = state.update((10.0, 90.0), ms(30)).expect("active");
        assert_eq!(update.delta, 160.0 + SWIPE_DEAD_ZONE);
        assert_eq!(update.axis, Axis::Horizontal);
    }

    #[test]
    fn a_long_drag_commits() {
        let mut state = started(3);
        state.update((COMMIT_DISTANCE * 0.6, 0.0), ms(200));
        state.update((COMMIT_DISTANCE * 0.6, 0.0), ms(400));
        let release = state.end(false, ms(420)).expect("a gesture was running");
        assert_eq!(
            release.gesture,
            Some(SwipeGesture::LeftToRight(Fingers::Three))
        );
    }

    #[test]
    fn a_short_slow_drag_snaps_back() {
        let mut state = started(3);
        state.update((COMMIT_DISTANCE * 0.2, 0.0), ms(100));
        state.update((COMMIT_DISTANCE * 0.05, 0.0), ms(400));
        let release = state.end(false, ms(900)).expect("a gesture was running");
        assert_eq!(release.gesture, None);
    }

    #[test]
    fn a_short_fast_flick_commits_on_velocity_alone() {
        let mut state = started(4);
        // Well under the commit distance, but covered in 10 ms.
        let step = COMMIT_DISTANCE * 0.2;
        state.update((step, 0.0), ms(5));
        state.update((step, 0.0), ms(10));
        let release = state.end(false, ms(10)).expect("a gesture was running");
        assert!(release.velocity > COMMIT_VELOCITY, "{}", release.velocity);
        assert_eq!(
            release.gesture,
            Some(SwipeGesture::LeftToRight(Fingers::Four))
        );
    }

    #[test]
    fn the_release_velocity_is_signed_like_the_travel() {
        let mut state = started(4);
        state.update((-200.0, 0.0), ms(10));
        state.update((-200.0, 0.0), ms(20));
        let release = state.end(false, ms(20)).expect("a gesture was running");
        assert!(release.velocity < 0.0, "{}", release.velocity);
    }

    #[test]
    fn cancelling_never_fires_but_still_reports() {
        let mut state = started(4);
        state.update((0.0, -COMMIT_DISTANCE * 2.0), ms(10));
        let release = state.end(true, ms(20)).expect("a gesture was running");

        assert!(release.cancelled);
        assert_eq!(release.gesture, None);
        assert!(
            state.end(false, ms(30)).is_none(),
            "a cancelled gesture is fully reset"
        );
    }

    #[test]
    fn vertical_direction_matches_travel_sign() {
        let mut state = started(4);
        state.update((0.0, -COMMIT_DISTANCE * 1.2), ms(10));
        let release = state.end(false, ms(20)).expect("a gesture was running");
        assert_eq!(
            release.gesture,
            Some(SwipeGesture::BottomToTop(Fingers::Four)),
            "negative y is upward travel"
        );
    }

    #[test]
    fn a_gesture_that_never_locked_an_axis_reports_no_direction() {
        let mut state = started(3);
        state.update((2.0, 2.0), ms(10));
        let release = state.end(false, ms(20)).expect("a gesture was running");
        assert_eq!(release.gesture, None);
        assert_eq!(
            release.velocity, 0.0,
            "with no axis there is no direction to have a speed along"
        );
    }

    #[test]
    fn a_second_gesture_does_not_inherit_the_first() {
        let mut state = started(3);
        state.update((AXIS_LOCK_THRESHOLD * 4.0, 0.0), ms(10));
        state.end(false, ms(20));

        state.begin(4, ms(100));
        assert!(
            state
                .update((AXIS_LOCK_THRESHOLD * 0.2, 0.0), ms(110))
                .is_none(),
            "leftover travel would have locked the axis immediately"
        );
    }

    #[test]
    fn the_head_start_points_the_way_the_swipe_locked() {
        let mut state = started(3);
        let update = state.update((0.0, -30.0), ms(10)).expect("locked");
        assert_eq!(update.delta, -30.0 - SWIPE_DEAD_ZONE);
    }

    #[test]
    fn the_head_start_does_not_count_as_speed() {
        let mut caught_up = started(3);
        caught_up.update((30.0, 0.0), ms(100));
        let released = caught_up
            .end(false, ms(100))
            .expect("a gesture was running");
        assert!(
            (released.velocity - 300.0).abs() < 1.0,
            "{}",
            released.velocity
        );
    }

    #[test]
    fn the_head_start_counts_towards_the_commit_distance() {
        let mut state = started(3);
        state.update((COMMIT_DISTANCE - SWIPE_DEAD_ZONE * 0.9, 0.0), ms(500));
        let release = state.end(false, ms(900)).expect("a gesture was running");
        assert_eq!(
            release.gesture,
            Some(SwipeGesture::LeftToRight(Fingers::Three))
        );
    }
}

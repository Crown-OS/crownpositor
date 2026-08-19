//! Spring-driven scalar interpolation. A single critically-damped harmonic
//! oscillator, integrated with fixed sub-steps so the integrator stays stable
//! independent of the frame rate the compositor wakes us at.
//!
//! Each animated value carries its own position + velocity, and a target. Step
//! the value with [`Spring::step`] when the frame ticks; check
//! [`Spring::at_rest`] to know when to stop requesting frames.
//!
//! Pick a "feel" per widget by choosing a [`SpringProfile`]. The default
//! [`SpringProfile::SNAPPY`] settles in roughly 200 ms with no overshoot; the
//! softer [`SpringProfile::SMOOTH`] is what dropdown / picker-style widgets use
//! when a value slides across a distance and needs to look unhurried;
//! [`SpringProfile::GESTURE`] catches a value the user was dragging.
//!
//! That last case is what [`Spring::hold`] and
//! [`Spring::set_target_with_velocity`] exist for: while a gesture owns the
//! value the spring is pinned to it, and on release the gesture's measured
//! velocity becomes the spring's, so the motion out of the fingers has no seam
//! in it.

use std::time::Instant;

use config::AnimationProfile;

/// Fixed integrator sub-step.
const SUBSTEP: f32 = 1.0 / 240.0;
/// Largest dt the integrator will accept in one tick.
const MAX_DT: f32 = 1.0 / 30.0;
/// Settle thresholds.
const EPSILON_POS: f32 = 0.0005;
const EPSILON_VEL: f32 = 0.01;

/// Named stiffness/damping pair. `damping ≈ 2 * sqrt(stiffness)` keeps the
/// response critically damped — no overshoot, gentle ease-out.
#[derive(Debug, Clone, Copy)]
pub struct SpringProfile {
    pub stiffness: f32,
    pub damping: f32,
}

impl SpringProfile {
    /// Fast settle (~200 ms). Default for toggles and sliders — anything that
    /// tracks the pointer or reacts to a discrete tap.
    pub const SNAPPY: Self = Self {
        stiffness: 320.0,
        damping: 35.78,
    };
    /// Softer, wider-arc settle (~350 ms). Good for values that visibly slide
    /// across the widget — dropdown panels, hover highlights that traverse
    /// rows, checkmark position between selection changes.
    pub const SMOOTH: Self = Self {
        stiffness: 180.0,
        damping: 26.83,
    };
    /// Catches a released gesture (~350 ms). Deliberately softer than
    /// [`SNAPPY`](Self::SNAPPY): a spring far stiffer than the speeds fingers
    /// actually move at swamps the velocity handed to it, and the handoff stops
    /// being felt. Critically damped, so it never bounces on its own — only a
    /// hard flick with little distance left carries past the target at all.
    pub const GESTURE: Self = Self {
        stiffness: 200.0,
        damping: 28.284,
    };

    /// The user-facing animation setting, resolved to a feel. `None` means
    /// motion is switched off and values should jump to their target.
    pub fn from_config(profile: AnimationProfile) -> Option<Self> {
        match profile {
            AnimationProfile::None => None,
            AnimationProfile::Snappy => Some(Self::SNAPPY),
            AnimationProfile::Standard => Some(Self::GESTURE),
            AnimationProfile::Smooth => Some(Self::SMOOTH),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub position: f32,
    pub velocity: f32,
    pub target: f32,
    pub profile: SpringProfile,
}

impl Spring {
    pub const fn new(value: f32) -> Self {
        Self::with_profile(value, SpringProfile::SNAPPY)
    }

    pub const fn with_profile(value: f32, profile: SpringProfile) -> Self {
        Self {
            position: value,
            velocity: 0.0,
            target: value,
            profile,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn set_profile(&mut self, profile: SpringProfile) {
        self.profile = profile;
    }

    /// Pins the value to something the user is dragging directly.
    ///
    /// The target follows the position so the spring exerts no force while the
    /// input owns the value, and the velocity is dropped because the gesture —
    /// not the integrator — is the one measuring how fast it is moving.
    pub fn hold(&mut self, position: f32) {
        self.position = position;
        self.target = position;
        self.velocity = 0.0;
    }

    /// Retarget while injecting a starting velocity — useful when the caller
    /// wants the spring to be *visibly* moving on the very next frame instead
    /// of ramping up from rest. Critically-damped springs otherwise cover
    /// only a few percent of the remaining distance in the first frame,
    /// which reads as "stuck" for close/dismiss animations.
    pub fn set_target_with_velocity(&mut self, target: f32, velocity: f32) {
        self.target = target;
        self.velocity = velocity;
    }

    /// Integrate the spring forward by `dt` seconds (clamped to MAX_DT).
    pub fn step(&mut self, dt: f32) {
        let mut remaining = dt.min(MAX_DT);
        while remaining > 0.0 {
            let h = remaining.min(SUBSTEP);
            let accel = -self.profile.stiffness * (self.position - self.target)
                - self.profile.damping * self.velocity;
            self.velocity += accel * h;
            self.position += self.velocity * h;
            remaining -= h;
        }
    }

    pub fn at_rest(&self) -> bool {
        (self.position - self.target).abs() < EPSILON_POS && self.velocity.abs() < EPSILON_VEL
    }

    #[allow(dead_code)]
    pub fn snap_to_target(&mut self) {
        self.position = self.target;
        self.velocity = 0.0;
    }
}

/// Dt source for animation loops.
///
/// Two ways to step it, one per backend style:
/// * [`Clock::tick`] — wall-clock delta since the previous call. For backends
///   without presentation feedback (winit).
/// * [`Clock::tick_to`] — delta between *target presentation times*. The KMS
///   backend passes the instant the frame being rendered will actually reach
///   the screen, so a spring's position is sampled at display time rather
///   than render time. Rendering jitter then stops being motion jitter.
pub struct Clock {
    last: Option<Instant>,
    /// The most recent target passed to `tick_to`, on the monotonic clock.
    last_target: Option<std::time::Duration>,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub const fn new() -> Self {
        Self {
            last: None,
            last_target: None,
        }
    }

    pub fn reset(&mut self) {
        self.last = None;
        self.last_target = None;
    }

    pub fn tick(&mut self) -> f32 {
        let now = Instant::now();
        let dt = self
            .last
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(1.0 / 60.0)
            .min(MAX_DT);
        self.last = Some(now);
        dt
    }

    /// Steps to a target presentation time (monotonic clock).
    ///
    /// A target older than the last one (a second output rendering for an
    /// earlier slot) contributes zero: springs already sit where that frame
    /// needs them, and stepping backwards would rewind visible motion.
    pub fn tick_to(&mut self, target: std::time::Duration) -> f32 {
        let dt = match self.last_target {
            Some(last) => target.saturating_sub(last).as_secs_f32().min(MAX_DT),
            None => 1.0 / 60.0,
        };
        if self.last_target.is_none_or(|last| target > last) {
            self.last_target = Some(target);
        }
        dt
    }
}

#[cfg(test)]
mod clock_tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn tick_to_measures_between_targets() {
        let mut clock = Clock::new();
        let base = Duration::from_secs(100);
        clock.tick_to(base);
        let dt = clock.tick_to(base + Duration::from_micros(16_667));
        assert!((dt - 0.016_667).abs() < 1e-6);
    }

    #[test]
    fn tick_to_never_steps_backwards() {
        let mut clock = Clock::new();
        let base = Duration::from_secs(100);
        clock.tick_to(base);
        // A second output rendering for an earlier slot.
        assert_eq!(clock.tick_to(base - Duration::from_millis(5)), 0.0);
        // And the anchor stays at the newest target.
        let dt = clock.tick_to(base + Duration::from_millis(10));
        assert!((dt - 0.010).abs() < 1e-6);
    }

    #[test]
    fn tick_to_clamps_long_gaps() {
        let mut clock = Clock::new();
        clock.tick_to(Duration::from_secs(100));
        // A VT switch later: one bounded step, not a teleport.
        let dt = clock.tick_to(Duration::from_secs(200));
        assert_eq!(dt, MAX_DT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps a spring at 60 Hz for at most `frames`, returning how many it took
    /// to settle — or `None` if it never did.
    fn settle(spring: &mut Spring, frames: usize) -> Option<usize> {
        (1..=frames).find(|_| {
            spring.step(1.0 / 60.0);
            spring.at_rest()
        })
    }

    #[test]
    fn converges_on_its_target() {
        let mut spring = Spring::new(0.0);
        spring.set_target(1.0);
        let frames = settle(&mut spring, 600).expect("spring never settled");
        assert!((spring.position - 1.0).abs() < EPSILON_POS);
        // SNAPPY is documented as settling in ~200 ms; allow generous slack but
        // catch a profile that has quietly become a crawl.
        assert!(frames < 60, "took {frames} frames at 60 Hz");
    }

    #[test]
    fn a_spring_that_never_moved_is_already_at_rest() {
        assert!(Spring::new(3.0).at_rest());
    }

    #[test]
    fn is_not_at_rest_until_both_position_and_velocity_have_settled() {
        let mut spring = Spring::new(0.0);
        spring.set_target(1.0);
        spring.step(1.0 / 60.0);
        assert!(!spring.at_rest(), "moving spring reported at rest");

        // Sitting on the target while still travelling is not rest either.
        let mut coasting = Spring::new(1.0);
        coasting.set_target_with_velocity(1.0, 5.0);
        assert!(!coasting.at_rest());
    }

    #[test]
    fn the_integrator_survives_a_stalled_frame() {
        // A dt far past MAX_DT — a compositor stall, a laptop resuming — must
        // not blow the explicit integrator up.
        let mut spring = Spring::with_profile(0.0, SpringProfile::SMOOTH);
        spring.set_target(1.0);
        for _ in 0..40 {
            spring.step(5.0);
            assert!(spring.position.is_finite(), "position diverged");
            assert!(spring.velocity.is_finite(), "velocity diverged");
            assert!(
                spring.position.abs() <= 2.0,
                "overshot to {}",
                spring.position
            );
        }
        assert!(spring.at_rest());
    }

    #[test]
    fn injected_velocity_moves_visibly_on_the_first_step() {
        let mut from_rest = Spring::new(1.0);
        from_rest.set_target(0.0);
        from_rest.step(1.0 / 60.0);

        let mut kicked = Spring::new(1.0);
        kicked.set_target_with_velocity(0.0, -3.0);
        kicked.step(1.0 / 60.0);

        assert!(
            kicked.position < from_rest.position,
            "kicked {} did not lead {}",
            kicked.position,
            from_rest.position
        );
        // "Visibly" means a good fraction of the distance, not a few percent.
        assert!(
            1.0 - kicked.position > 0.03,
            "moved only {}",
            1.0 - kicked.position
        );
    }

    #[test]
    fn a_released_flick_carries_its_own_speed_into_the_spring() {
        // Magnitudes a touchpad actually produces: a fifth of a page left to
        // travel, fingers leaving at about two pages a second. If the profile
        // is stiff enough to swamp that, the handoff is invisible and there was
        // no point measuring the velocity at all.
        let mut flicked = Spring::with_profile(0.8, SpringProfile::GESTURE);
        flicked.set_target_with_velocity(1.0, 2.0);

        let mut from_rest = Spring::with_profile(0.8, SpringProfile::GESTURE);
        from_rest.set_target(1.0);

        flicked.step(1.0 / 60.0);
        from_rest.step(1.0 / 60.0);
        let with_speed = flicked.position - 0.8;
        let unaided = from_rest.position - 0.8;
        assert!(
            with_speed > unaided * 1.5,
            "the flick barely led: {with_speed} against {unaided}"
        );

        // And it still lands, rather than trading convergence for the kick.
        for _ in 0..120 {
            flicked.step(1.0 / 60.0);
        }
        assert!(flicked.at_rest() && (flicked.position - 1.0).abs() < EPSILON_POS);
    }

    #[test]
    fn a_critically_damped_spring_never_bounces_on_its_own() {
        let mut spring = Spring::with_profile(0.0, SpringProfile::GESTURE);
        spring.set_target(1.0);
        for _ in 0..120 {
            spring.step(1.0 / 60.0);
            assert!(spring.position <= 1.0, "overshot to {}", spring.position);
        }
        assert!(spring.at_rest());
    }

    #[test]
    fn holding_a_value_freezes_the_spring_on_it() {
        let mut spring = Spring::new(0.0);
        spring.set_target_with_velocity(10.0, 50.0);
        spring.hold(3.5);

        assert!(spring.at_rest(), "a held spring exerts no force");
        spring.step(1.0 / 60.0);
        assert_eq!(spring.position, 3.5, "it must not drift under the fingers");
    }

    #[test]
    fn every_profile_is_critically_damped() {
        // `damping = 2√k` is the line between a spring that eases in and one
        // that rings. A profile that drifts off it should be a deliberate
        // choice, not a typo in a literal.
        for profile in [
            SpringProfile::SNAPPY,
            SpringProfile::SMOOTH,
            SpringProfile::GESTURE,
        ] {
            let critical = 2.0 * profile.stiffness.sqrt();
            assert!(
                (profile.damping - critical).abs() < 0.01,
                "{profile:?} damps at {}, critical is {critical}",
                profile.damping
            );
        }
    }

    #[test]
    fn disabling_animations_resolves_to_no_profile() {
        assert!(SpringProfile::from_config(AnimationProfile::None).is_none());
        for profile in [
            AnimationProfile::Snappy,
            AnimationProfile::Standard,
            AnimationProfile::Smooth,
        ] {
            assert!(SpringProfile::from_config(profile).is_some(), "{profile:?}");
        }
    }

    #[test]
    fn snap_to_target_settles_immediately() {
        let mut spring = Spring::new(0.0);
        spring.set_target_with_velocity(10.0, 100.0);
        spring.snap_to_target();
        assert_eq!(spring.position, 10.0);
        assert!(spring.at_rest());
    }

    #[test]
    fn clock_clamps_its_first_and_longest_frames() {
        let mut clock = Clock::new();
        assert_eq!(clock.tick(), 1.0 / 60.0);
        assert!(clock.tick() <= MAX_DT);
        clock.reset();
        assert_eq!(clock.tick(), 1.0 / 60.0);
    }
}

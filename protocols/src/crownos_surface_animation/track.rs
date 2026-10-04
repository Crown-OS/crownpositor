//! One property's motion: at rest on a value, or springing toward a target.

use std::time::Duration;

use motion::{Animation, Spring};

/// The value and velocity a running animation had when a new one replaced it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interruption {
    pub value: f32,
    pub velocity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    value: f32,
    motion: Option<Animation<f32>>,
}

impl Track {
    pub const fn resting(value: f32) -> Self {
        Self {
            value,
            motion: None,
        }
    }

    /// What the renderer draws: the value sampled on the last frame.
    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn is_moving(&self) -> bool {
        self.motion.is_some()
    }

    pub fn set(&mut self, value: f32) {
        *self = Self::resting(value);
    }

    /// Springs toward `target` from `start`, continuing a running animation's
    /// value and velocity at that instant rather than jumping.
    pub fn animate(
        &mut self,
        target: f32,
        spring: Spring,
        start: Duration,
    ) -> Option<Interruption> {
        let Some(running) = self.motion else {
            self.motion = Some(Animation::new(
                self.value,
                target,
                0.0,
                start,
                spring.into(),
            ));
            return None;
        };
        let handoff = running.sample(start);
        self.motion = Some(running.retarget(start, target, spring.into()));
        Some(Interruption {
            value: handoff.value,
            velocity: handoff.velocity,
        })
    }

    /// Samples the motion at `now`, returning the resting value on the frame it
    /// settles.
    pub fn advance(&mut self, now: Duration) -> Option<f32> {
        let sample = self.motion?.sample(now);
        self.value = sample.value;
        if !sample.finished {
            return None;
        }
        self.motion = None;
        Some(sample.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Duration = Duration::from_secs(100);

    fn millis(offset: u64) -> Duration {
        START + Duration::from_millis(offset)
    }

    #[test]
    fn an_animation_starts_from_the_resting_value() {
        let mut track = Track::resting(10.0);
        assert_eq!(track.animate(110.0, Spring::smooth(), START), None);
        assert_eq!(track.advance(START), None);
        assert_eq!(track.value(), 10.0);
        assert!(track.is_moving());
    }

    #[test]
    fn settling_reports_the_target_once_and_stops() {
        let mut track = Track::resting(0.0);
        track.animate(100.0, Spring::snappy(), START);
        assert_eq!(track.advance(millis(5_000)), Some(100.0));
        assert!(!track.is_moving());
        assert_eq!(track.advance(millis(6_000)), None);
        assert_eq!(track.value(), 100.0);
    }

    #[test]
    fn retargeting_continues_from_the_sampled_value_and_velocity() {
        let mut track = Track::resting(0.0);
        track.animate(100.0, Spring::snappy(), START);
        let reference = Animation::new(0.0_f32, 100.0, 0.0, START, Spring::snappy().into());
        let expected = reference.sample(millis(80));

        let interruption = track.animate(-50.0, Spring::snappy(), millis(80));

        assert_eq!(
            interruption,
            Some(Interruption {
                value: expected.value,
                velocity: expected.velocity,
            })
        );
        track.advance(millis(80));
        assert_eq!(track.value(), expected.value);
        track.advance(millis(81));
        let drift = (track.value() - expected.value).abs();
        assert!(
            drift < 1.0,
            "position jumped by {drift} across the retarget"
        );
    }

    #[test]
    fn a_set_cancels_the_motion() {
        let mut track = Track::resting(0.0);
        track.animate(100.0, Spring::smooth(), START);
        track.set(42.0);
        assert!(!track.is_moving());
        assert_eq!(track.advance(millis(50)), None);
        assert_eq!(track.value(), 42.0);
    }
}

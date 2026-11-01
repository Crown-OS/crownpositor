//! One property of a morph on a spring of its own, which may wait a beat
//! before it starts.

use spacecontrol::animations::spring::{Spring, SpringProfile};

/// How one channel moves on its way to an end of the morph.
#[derive(Debug, Clone, Copy)]
pub struct Beat {
    pub profile: SpringProfile,
    /// Seconds a channel standing still waits before it sets off.
    pub delay: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Channel {
    spring: Spring,
    /// Whether it stood still when the morph last turned, so it waits out
    /// its beat's delay instead of turning in flight.
    waiting: bool,
}

impl Channel {
    pub const fn at(position: f32, profile: SpringProfile) -> Self {
        Self {
            spring: Spring::with_profile(position, profile),
            waiting: false,
        }
    }

    /// Heads for `target`. Only a channel at rest waits for its delay: one
    /// already moving turns at once, carrying its speed into the turn.
    pub fn head_for(&mut self, target: f32) {
        self.waiting = self.spring.at_rest();
        self.spring.set_target(target);
    }

    /// Advances it by `dt`, `elapsed` seconds after the morph turned.
    pub fn step(&mut self, dt: f32, elapsed: f32, beat: Beat) {
        self.spring.set_profile(beat.profile);
        let moving_for = match self.waiting {
            true => (elapsed - beat.delay).min(dt),
            false => dt,
        };
        if moving_for <= 0.0 {
            return;
        }
        self.waiting = false;
        self.spring.step(moving_for);
    }

    pub fn position(&self) -> f32 {
        self.spring.position
    }

    pub fn velocity(&self) -> f32 {
        self.spring.velocity
    }

    pub fn target(&self) -> f32 {
        self.spring.target
    }

    pub fn at_rest(&self) -> bool {
        self.spring.at_rest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 120.0;
    const PROFILE: SpringProfile = SpringProfile::SNAPPY;

    fn beat(delay: f32) -> Beat {
        Beat {
            profile: PROFILE,
            delay,
        }
    }

    #[test]
    fn a_channel_at_rest_waits_out_its_delay() {
        let mut channel = Channel::at(0.0, PROFILE);
        channel.head_for(1.0);
        let mut elapsed = 0.0;
        while elapsed + FRAME < 0.1 {
            elapsed += FRAME;
            channel.step(FRAME, elapsed, beat(0.1));
            assert_eq!(channel.position(), 0.0);
        }
        elapsed += 2.0 * FRAME;
        channel.step(2.0 * FRAME, elapsed, beat(0.1));
        assert!(channel.position() > 0.0);
    }

    #[test]
    fn a_moving_channel_turns_at_once_keeping_its_speed() {
        let mut channel = Channel::at(0.0, PROFILE);
        channel.head_for(1.0);
        for frame in 1..=10 {
            channel.step(FRAME, frame as f32 * FRAME, beat(0.0));
        }
        let (position, velocity) = (channel.position(), channel.velocity());
        channel.head_for(0.0);
        channel.step(FRAME, FRAME, beat(0.5));
        assert!(channel.position() > position, "it froze instead of turning");
        assert!(channel.velocity() < velocity);
    }
}

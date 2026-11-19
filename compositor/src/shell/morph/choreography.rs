//! How the channels of a morph move against one another.
//!
//! The liquid choreography is fitted, channel by channel, to a recording of
//! the glass button-to-sheet morph it imitates, then played 30% faster. The
//! shape's length rides the leading edge's spring, so it never squashes or
//! stretches along the way it travels. Growing, it shoots out first and
//! overshoots the window by about 7% of its journey, off the screen and over
//! the bar when the window is near the top, while the breadth follows on a
//! softer spring. Shrinking runs the other way round: the breadth collapses
//! first and the shape only travels once it has.

use config::MinimizeAnimation;
use spacecontrol::animations::spring::SpringProfile;

use super::{Channel, MorphEnd, channel::Beat};

/// One spring per channel for one direction of travel.
#[derive(Debug, Clone, Copy)]
pub struct Moves {
    /// The edge farthest from the icon, along the axis of travel.
    pub travel: Beat,
    /// The extent along that axis.
    pub length: Beat,
    /// The extent and position across it.
    pub breadth: Beat,
}

#[derive(Debug, Clone, Copy)]
pub struct Choreography {
    pub grow: Moves,
    pub shrink: Moves,
    /// How much of the leading edge's lead its own breadth takes on: what
    /// turns the shape into a trapezoid in flight.
    pub taper: f64,
    /// How much of an overshoot past either end the shape's length takes on:
    /// below 1 the shape lifts past its window instead of stretching.
    pub stretch: f64,
    /// Seconds a morph that carried its icon home stays drawn on it, while
    /// the launcher puts its own icon back.
    pub linger: f32,
}

impl Choreography {
    /// Every channel on one spring: the shape stays a rectangle.
    pub const UNIFORM: Self = Self {
        grow: Moves::together(spring(0.36, 0.92)),
        shrink: Moves::together(spring(0.30, 0.92)),
        taper: 0.0,
        stretch: 1.0,
        linger: 0.0,
    };

    pub const LIQUID: Self = Self {
        grow: Moves {
            travel: LIQUID_GROW_TRAVEL,
            length: LIQUID_GROW_TRAVEL,
            breadth: beat(0.43, 0.94, 0.055),
        },
        shrink: Moves {
            travel: LIQUID_SHRINK_TRAVEL,
            length: LIQUID_SHRINK_TRAVEL,
            breadth: beat(0.21, 1.24, 0.0),
        },
        taper: 0.15,
        stretch: 0.3,
        linger: 0.1,
    };

    pub fn of(effect: MinimizeAnimation) -> Self {
        match effect {
            MinimizeAnimation::Liquid => Self::LIQUID,
            MinimizeAnimation::Genie | MinimizeAnimation::Scale => Self::UNIFORM,
        }
    }

    pub fn moves(&self, toward: MorphEnd) -> &Moves {
        match toward {
            MorphEnd::Window => &self.grow,
            MorphEnd::Icon => &self.shrink,
        }
    }

    /// `length` with only `stretch` of its overshoot past either end.
    pub(super) fn soften_stretch(&self, length: f64) -> f64 {
        let overshoot = length - length.clamp(0.0, 1.0);
        length - overshoot * (1.0 - self.stretch)
    }

    /// The channels at rest on `end`.
    pub(super) fn resting_on(&self, end: MorphEnd) -> [Channel; 3] {
        let position = match end {
            MorphEnd::Window => 1.0,
            MorphEnd::Icon => 0.0,
        };
        let moves = self.moves(end);
        [moves.travel, moves.length, moves.breadth].map(|beat| Channel::at(position, beat.profile))
    }
}

impl Moves {
    const fn together(profile: SpringProfile) -> Self {
        let beat = Beat {
            profile,
            delay: 0.0,
        };
        Self {
            travel: beat,
            length: beat,
            breadth: beat,
        }
    }
}

const LIQUID_GROW_TRAVEL: Beat = beat(0.34, 0.62, 0.0);
const LIQUID_SHRINK_TRAVEL: Beat = beat(0.38, 0.86, 0.090);

const fn beat(response_seconds: f32, damping_ratio: f32, delay: f32) -> Beat {
    Beat {
        profile: spring(response_seconds, damping_ratio),
        delay,
    }
}

/// A spring by its period and damping ratio, the way designers specify one.
const fn spring(response_seconds: f32, damping_ratio: f32) -> SpringProfile {
    let angular_frequency = std::f32::consts::TAU / response_seconds;
    SpringProfile {
        stiffness: angular_frequency * angular_frequency,
        damping: 2.0 * damping_ratio * angular_frequency,
    }
}

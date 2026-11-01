//! How the channels of a morph move against one another.
//!
//! The liquid choreography is fitted, channel by channel, to a recording of
//! the glass button-to-sheet morph it imitates. Growing, the leading edge
//! shoots out first and overshoots while the shape's length and breadth
//! follow on softer springs, so the trailing edge is dragged along before it
//! settles. Shrinking runs the other way round: the shape collapses first and
//! the leading edge only follows once it has.

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
        linger: 0.0,
    };

    pub const LIQUID: Self = Self {
        grow: Moves {
            travel: beat(0.48, 0.58, 0.0),
            length: beat(0.74, 0.72, 0.040),
            breadth: beat(0.62, 0.84, 0.080),
        },
        shrink: Moves {
            travel: beat(0.54, 0.72, 0.130),
            length: beat(0.60, 0.92, 0.005),
            breadth: beat(0.30, 1.24, 0.0),
        },
        taper: 0.15,
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

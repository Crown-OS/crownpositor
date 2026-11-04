//! How the switcher moves.

use spacecontrol::animations::spring::SpringProfile;

/// The stack coming up: a little under critical, so it overshoots its resting
/// place by a hair and settles back — the one bounce that reads as physical.
pub const RISE: SpringProfile = SpringProfile {
    stiffness: 260.0,
    damping: 25.8,
};

/// The stack going. Critically damped: the top card landing back on its
/// window must not overshoot it.
pub const SINK: SpringProfile = SpringProfile {
    stiffness: 400.0,
    damping: 40.0,
};

/// The stack sliding to a new top card, ~250 ms to arrive.
pub const SELECT: SpringProfile = SpringProfile {
    stiffness: 500.0,
    damping: 44.72,
};

/// A card coming out of the stack to the top, or going back into it.
pub const FOCUS: SpringProfile = SpringProfile {
    stiffness: 260.0,
    damping: 32.25,
};

/// Entries arriving and leaving, and the hover lift.
pub const ENTRY: SpringProfile = SpringProfile::SNAPPY;

/// How large the stack starts as it comes up, and ends as it goes.
pub const ARRIVAL_SCALE: f64 = 0.9;

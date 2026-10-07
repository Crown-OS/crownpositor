//! How the switcher moves.

use spacecontrol::animations::spring::SpringProfile;

/// The strip rising: a little under critical, so it overshoots its resting
/// place by a hair and settles back — the one bounce that reads as physical.
pub const RISE: SpringProfile = SpringProfile {
    stiffness: 260.0,
    damping: 25.8,
};

/// The strip sinking and the chosen window zooming forward. Critically damped:
/// a window landing on its own frame must not overshoot it.
pub const SINK: SpringProfile = SpringProfile {
    stiffness: 400.0,
    damping: 40.0,
};

/// The row gliding to a new selection, ~150 ms to arrive.
pub const SELECT: SpringProfile = SpringProfile {
    stiffness: 1000.0,
    damping: 63.25,
};

/// Entries arriving and leaving, and the hover lift.
pub const ENTRY: SpringProfile = SpringProfile::SNAPPY;

/// How far the workspace behind the strip steps back, as a scale factor.
pub const SCALE_BACK: f64 = 0.94;

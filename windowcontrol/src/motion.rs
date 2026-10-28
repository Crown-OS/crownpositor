//! How the switcher moves.

use spacecontrol::animations::spring::SpringProfile;

/// The strip rising: a little under critical, so it overshoots its resting
/// place by a hair and settles back — the one bounce that reads as physical.
pub const RISE: SpringProfile = SpringProfile {
    stiffness: 260.0,
    damping: 25.8,
};

/// The strip sinking. Critically damped: the workspace stepping back into
/// place must not overshoot it.
pub const SINK: SpringProfile = SpringProfile {
    stiffness: 400.0,
    damping: 40.0,
};

/// The row gliding to a new selection, ~250 ms to arrive.
pub const SELECT: SpringProfile = SpringProfile {
    stiffness: 500.0,
    damping: 44.72,
};

/// A card and its window coming forward as the selection, or stepping back.
pub const FOCUS: SpringProfile = SpringProfile {
    stiffness: 260.0,
    damping: 32.25,
};

/// Entries arriving and leaving, and the hover lift.
pub const ENTRY: SpringProfile = SpringProfile::SNAPPY;

/// How far the windows behind the strip step back, as a scale factor.
pub const SCALE_BACK: f64 = 0.94;

/// How opaque the windows behind the strip are while another is selected.
pub const FADE_BACK: f64 = 0.3;

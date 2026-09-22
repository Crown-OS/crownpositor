//! SpaceControl: the mission-control overview.
//!
//! Everything the feature needs lives here — the grid solver, the state
//! machine that opens and closes it, hit-testing, drag and drop, and the render
//! elements it draws. The compositor owns the windows; this crate is told about
//! them through plain snapshots and hands back geometry, so nothing here
//! depends on the shell's internals.
//!
//! [`animations`] is the exception, and the reason the compositor depends on
//! this crate rather than the other way round: the springs the overview runs on
//! are the same ones tiles and the workspace viewport already use.

pub mod animations;
pub mod interaction;
pub mod layout;
pub mod overview;
pub mod render;
pub mod scene;

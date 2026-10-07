//! SpaceControl: the mission-control overview.
//!
//! The feature's model lives here — the grid solver, the state machine that
//! opens and closes it, hit-testing, drag and drop. The compositor owns the
//! windows and draws them; this crate is told about them through plain
//! snapshots and hands back geometry, so nothing here depends on the shell or
//! on a renderer.
//!
//! [`animations`] is the exception, and the reason the compositor depends on
//! this crate rather than the other way round: the springs the overview runs on
//! are the same ones tiles and the workspace viewport already use.

pub mod animations;
pub mod interaction;
pub mod layout;
pub mod overview;
pub mod scene;

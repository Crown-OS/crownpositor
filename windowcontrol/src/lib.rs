//! WindowControl: the Alt+Tab switcher.
//!
//! A glass strip rises from the bottom edge with the active workspace's
//! windows in it, most recent first. This crate is the model — which entry is
//! selected, where every thumbnail sits, how far the strip has risen — keyed by
//! whatever the compositor calls a window. Drawing and input routing stay in
//! the compositor, the same split as SpaceControl.

pub mod layout;
pub mod motion;
pub mod strip;
pub mod switcher;

pub use layout::{Metrics, Slot};
pub use strip::Direction;
pub use switcher::Switcher;

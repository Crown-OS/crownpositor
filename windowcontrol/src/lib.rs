//! WindowControl: the Alt+Tab switcher.
//!
//! A glass panel rises from the bottom edge with the active workspace's
//! windows stacked in it as cards, most recent first, while the selected
//! window itself comes forward on the desktop behind. This crate is the model — which entry is
//! selected, where every thumbnail sits, how far the strip has risen — keyed by
//! whatever the compositor calls a window. Drawing and input routing stay in
//! the compositor, the same split as SpaceControl.

pub mod layout;
pub mod motion;
pub mod strip;
pub mod switcher;

pub use layout::{Metrics, Panel, Slot};
pub use strip::Direction;
pub use switcher::Switcher;

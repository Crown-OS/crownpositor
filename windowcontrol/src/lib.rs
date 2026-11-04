//! WindowControl: the Alt+Tab switcher.
//!
//! The active workspace's windows come up as a stack of cards over the frosted
//! desktop, most recent first: the selected window on top in the middle, the
//! rest stacked behind it to either side. This crate is the model — which
//! entry is selected, where every card sits, how far the stack has come up —
//! keyed by whatever the compositor calls a window. Drawing and input routing
//! stay in the compositor, the same split as SpaceControl.

pub mod layout;
pub mod motion;
pub mod strip;
pub mod switcher;

pub use layout::{Metrics, Slot};
pub use strip::Direction;
pub use switcher::Switcher;

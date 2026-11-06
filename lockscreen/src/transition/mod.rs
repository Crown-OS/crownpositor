//! How the lock screen moves: one fixed choreography, carried by a damped
//! spring rather than timed.
//!
//! A host never animates anything itself: it asks the
//! [`LockFlow`](crate::flow::LockFlow) for the [`Frame`] of an instant and
//! draws with it. A still of either resting stage is
//! [`Moments::CLOCK`] or [`Moments::PROMPT`] turned into a frame.

mod choreography;
mod frame;
mod spring;
mod style;

pub use choreography::{LAYER_BLUR_REACH, Moments, PROMPT_BLUR, PROMPT_DIM};
pub use frame::Frame;
pub use spring::{Spring, SpringValue};
pub use style::LayerStyle;

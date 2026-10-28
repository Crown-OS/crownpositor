//! The CrownOS lock screen, minus the compositor: what it is doing, how it
//! moves, where everything sits and what it looks like.
//!
//! The compositor drives a [`LockFlow`](flow::LockFlow) with input, checks the
//! password it hands out through [`auth`], and composites the sprites
//! [`paint`] produces with the [`Frame`](flow::Frame) styles on the GPU.
//! Settings uses the same layout, paint and transitions for its preview, so
//! what it shows is what the lock screen draws.

#[cfg(feature = "pam")]
pub mod auth;
pub mod clock;
pub mod flow;
pub mod image;
pub mod layout;
pub mod paint;
pub mod palette;
pub mod secret;
pub mod transition;
pub mod user;
mod util;

pub use cosmic_text;
pub use lockscene;
pub use tiny_skia;

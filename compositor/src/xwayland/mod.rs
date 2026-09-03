//! X11 client support, by way of `xwayland-satellite`.
//!
//! The bridge runs Xwayland in its own process and re-presents X11 windows as
//! ordinary `xdg_toplevel`s, so nothing in the shell, the layout, the renderer
//! or the focus model has to know that X11 exists. What is left for the
//! compositor is owning the display slot, deciding when to start the bridge,
//! and telling its children where to find it.

mod activation;
mod satellite;
mod socket;

pub use activation::Xwayland;

//! Server-side `wp_color_management_v1` (staging, version 3).

pub mod capabilities;
pub mod creator;
mod dispatch;
pub mod info;
pub mod math;
pub mod record;
pub mod registry;
pub mod state;

pub use info::send as send_image_description_info;
pub use state::{
    ColorManagementHandler, ColorManagementState, SurfaceColor, color_of, surface_color,
};

/// The generated bindings, so `delegate_color_management!` can name every
/// interface through `$crate`.
#[doc(hidden)]
pub mod reexports {
    pub use wayland_protocols::wp::color_management::v1::server::*;
}

//! Deciding whether an output needs the colour pipeline, and what each surface
//! needs within it.
//!
//! The whole design turns on one rule: an output where every surface is
//! ordinary sRGB must render *exactly* as it did before colour management
//! existed — same program, same format, same element list. Colour management
//! is not a tax on the common case.
//!
//! So the scan below runs every frame and is deliberately cheap.

use protocols::color_management::color_of;
use smithay::{
    desktop::layer_map_for_output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::seat::WaylandFocus,
};

use crate::{
    color::pipeline::SurfaceTransform,
    shell::{Shell, monitor::Monitor},
};

/// Whether this output's frame needs colour management at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorPass {
    /// Nothing on the output is described, so the pre-colour-management path
    /// is used unchanged.
    #[default]
    Passthrough,
    /// Something needs decoding, so every surface goes through the colour
    /// program — including the undescribed ones, which take the protocol's
    /// defined sRGB default.
    Managed,
}

impl ColorPass {
    pub fn is_managed(self) -> bool {
        self == Self::Managed
    }
}

/// Whether anything visible on this output carries an image description.
///
/// A surface that describes itself as plain sRGB still counts: the compositor
/// cannot tell by eye, and honouring the description is the point.
pub fn output_needs_color(shell: &Shell, monitor: &Monitor) -> bool {
    let windows = shell
        .visible_windows(monitor)
        .filter_map(|tile| tile.window().wl_surface())
        .any(|surface| is_described(&surface));
    if windows {
        return true;
    }

    let map = layer_map_for_output(monitor.output());
    map.layers()
        .any(|layer| is_described(layer.wl_surface()))
}

fn is_described(surface: &WlSurface) -> bool {
    color_of(surface).is_some()
}

/// The transform for one surface, in a frame that is colour managed.
///
/// An undescribed surface gets the protocol's default rather than being left
/// alone: once the frame is being composited in linear light, *everything* has
/// to be decoded into it or the undescribed windows come out wrong.
pub fn transform_for(surface: Option<&WlSurface>) -> SurfaceTransform {
    let described = surface
        .and_then(color_of)
        .and_then(|color| SurfaceTransform::for_description(&color.description));

    described.unwrap_or_else(SurfaceTransform::default_srgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undescribed_surface_takes_the_protocols_srgb_default() {
        assert_eq!(transform_for(None), SurfaceTransform::default_srgb());
    }

    #[test]
    fn the_default_pass_is_the_one_that_changes_nothing() {
        assert_eq!(ColorPass::default(), ColorPass::Passthrough);
        assert!(!ColorPass::default().is_managed());
    }
}

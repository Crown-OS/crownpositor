//! Which surfaces have ever been given an image description.
//!
//! Only these can need converting, so a frame looks at them alone instead of
//! every surface on screen: a session where no client speaks colour
//! management pays nothing at all.

use protocols::color_management::color_of;
use smithay::{
    backend::renderer::element::Id,
    reexports::wayland_server::{Resource, Weak, protocol::wl_surface::WlSurface},
};

use crate::{
    color::pipeline::SurfaceTransform,
    rendering::color::{ColorConversions, Conversion},
};

#[derive(Debug, Default)]
pub struct DescribedSurfaces(Vec<Weak<WlSurface>>);

impl DescribedSurfaces {
    pub fn track(&mut self, surface: &WlSurface) {
        self.0.retain(Weak::is_alive);
        if !self.0.iter().any(|tracked| tracked.id() == surface.id()) {
            self.0.push(surface.downgrade());
        }
    }

    /// What each surface's committed description needs this frame. A surface
    /// already in the output encoding is left out, so it draws as an
    /// undescribed one does.
    pub fn conversions(&self) -> ColorConversions {
        self.0
            .iter()
            .filter_map(|tracked| tracked.upgrade().ok())
            .filter_map(|surface| {
                let color = color_of(&surface)?;
                let transform = SurfaceTransform::for_description(&color.description)?;
                (!transform.is_passthrough()).then(|| {
                    let conversion = Conversion {
                        transform,
                        description: color.description.id().get(),
                    };
                    (Id::from_wayland_resource(&surface), conversion)
                })
            })
            .collect()
    }
}

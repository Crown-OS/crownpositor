//! Whether a CRTC can flip asynchronously through the atomic API, and the
//! handles an async commit needs.

use smithay::{
    backend::drm::DrmSurface,
    reexports::drm::{
        Device as _, DriverCapability,
        control::{plane, property},
    },
};

use crate::backend::kms::props::property_handle;

#[derive(Debug, Clone, Copy)]
pub struct AsyncCaps {
    pub plane: plane::Handle,
    pub fb_id: property::Handle,
}

impl AsyncCaps {
    /// `None` when the driver cannot do atomic async flips (kernel 6.8+, and
    /// not every driver), or the surface is on the legacy API.
    pub fn probe(surface: &DrmSurface) -> Option<Self> {
        if surface.is_legacy() {
            return None;
        }
        let supported = surface
            .get_driver_capability(DriverCapability::AtomicASyncPageFlip)
            .is_ok_and(|value| value == 1);
        if !supported {
            return None;
        }
        let plane = surface.plane();
        Some(Self {
            plane,
            fb_id: property_handle(surface, plane, "FB_ID")?,
        })
    }
}

//! One GPU: its DRM device, buffer allocation and connector bookkeeping.

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::{
            dmabuf::DmabufAllocator,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{DrmDevice, DrmDeviceFd, DrmNode},
    },
    reexports::{calloop::RegistrationToken, drm::control::crtc},
};
use smithay_drm_extras::drm_scanner::DrmScanner;

use crate::backend::{
    kms::{surface::Surface, vulkan::VulkanContext},
    render::{CrownAllocator, GraphicsApi},
};

/// A GPU the session has opened.
pub struct Device {
    /// The node the device was opened through (primary/card node).
    pub node: DrmNode,
    /// The node rendering happens on. Falls back to `node` for split
    /// display/render SoCs where the query fails.
    pub render_node: DrmNode,
    pub drm: DrmDevice,
    pub gbm: GbmDevice<DrmDeviceFd>,
    /// Tracks which connectors appeared/disappeared between udev `Changed`
    /// events and assigns CRTCs to them.
    pub scanner: DrmScanner,
    /// One rendering surface per connected monitor.
    pub surfaces: HashMap<crtc::Handle, Surface>,
    /// The DRM event source (vblanks) in the event loop, removed when the
    /// device is unplugged.
    pub drm_token: RegistrationToken,
}

impl Device {
    /// The scanout-buffer allocator for one surface on this GPU.
    ///
    /// Requesting Vulkan without a usable Vulkan device *degrades* to GBM
    /// with a warning rather than failing the output: an output that lights
    /// up on the fallback path beats a black screen on the preferred one.
    pub fn create_allocator(
        &self,
        api: GraphicsApi,
        vulkan: Option<&VulkanContext>,
    ) -> CrownAllocator {
        if api == GraphicsApi::Vulkan {
            match vulkan.map(|context| context.allocator_for_node(&self.render_node)) {
                Some(Ok(allocator)) => {
                    return CrownAllocator::Vulkan(DmabufAllocator(allocator));
                }
                Some(Err(err)) => {
                    tracing::warn!(%err, node = %self.render_node, "vulkan allocation unavailable, using GBM");
                }
                None => {
                    tracing::warn!("vulkan requested but no instance exists, using GBM");
                }
            }
        }

        CrownAllocator::Gbm(DmabufAllocator(GbmAllocator::new(
            self.gbm.clone(),
            // Scanned out by the CRTC *and* rendered into by GLES.
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        )))
    }
}

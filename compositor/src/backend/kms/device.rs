//! One GPU: its DRM device, buffer allocation and connector bookkeeping.

use std::collections::{HashMap, HashSet};

use smithay::{
    wayland::drm_lease::{DrmLease, DrmLeaseState},
    backend::{
        allocator::{
            dmabuf::DmabufAllocator,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{DrmDevice, DrmDeviceFd, DrmNode},
    },
    reexports::{calloop::RegistrationToken, drm::control::{connector, crtc}},
};
use smithay_drm_extras::drm_scanner::DrmScanner;

use crate::backend::{
    kms::{head::Head, surface::Surface, vulkan::VulkanContext},
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
    /// Every connector that has something plugged into it, lit or not.
    ///
    /// The shell only knows about outputs that are *on*, so this is the only
    /// record of a monitor the user has switched off — and therefore the only
    /// way they can switch it back on.
    pub heads: HashMap<connector::Handle, Head>,
    /// One rendering surface per connected monitor.
    pub surfaces: HashMap<crtc::Handle, Surface>,
    /// The DRM event source (vblanks) in the event loop, removed when the
    /// device is unplugged.
    pub drm_token: RegistrationToken,
    /// `wp_drm_lease_device_v1` for this GPU, when it could be created.
    pub lease_state: Option<DrmLeaseState>,
    /// Leases handed out, by their protocol id. Dropping one revokes it.
    pub active_leases: HashMap<u32, DrmLease>,
    /// CRTCs a client is driving. Not the compositor's to give to a monitor.
    pub leased_crtcs: HashSet<crtc::Handle>,
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

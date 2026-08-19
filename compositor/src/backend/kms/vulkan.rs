//! The Vulkan side of the graphics-API switch.
//!
//! On smithay 0.7 this means *allocation*: scanout buffers come out of a
//! [`VulkanAllocator`] as dmabufs and are drawn into by GLES3 through EGL
//! import (see [`crate::backend::render`] for why). Everything Vulkan-specific
//! is contained here so a future Vulkan renderer replaces this file's callers,
//! not its callers' callers.
//!
//! No `unsafe` appears below: smithay's `Instance`/`PhysicalDevice`/
//! `VulkanAllocator` wrappers own the FFI and its invariants.

use smithay::backend::{
    allocator::vulkan::{ImageUsageFlags, VulkanAllocator},
    drm::DrmNode,
    vulkan::{version::Version, Instance, PhysicalDevice},
};

use crate::backend::render::RenderInitError;

/// A live Vulkan instance, created once per session and shared by every
/// GPU's allocator.
#[derive(Debug)]
pub struct VulkanContext {
    instance: Instance,
}

impl VulkanContext {
    /// Brings up the instance. Failing here is not fatal to the compositor:
    /// the caller falls back to GBM allocation and says so.
    pub fn try_new() -> Result<Self, RenderInitError> {
        // 1.2 is what VulkanAllocator's format-modifier queries want; every
        // driver young enough to sit under a Wayland session has it.
        let instance = Instance::new(Version::VERSION_1_2, None)
            .map_err(|err| RenderInitError::VulkanInstance(err.to_string()))?;
        Ok(Self { instance })
    }

    /// An allocator whose buffers live on the GPU behind `node`.
    ///
    /// The physical device is matched by DRM node id, because "the same GPU"
    /// is the only thing that makes a Vulkan-allocated dmabuf cheap for the
    /// GLES context on that node to import (no cross-device copy).
    pub fn allocator_for_node(&self, node: &DrmNode) -> Result<VulkanAllocator, RenderInitError> {
        let physical_device = PhysicalDevice::enumerate(&self.instance)
            .map_err(|err| RenderInitError::VulkanInstance(err.to_string()))?
            .find(|phd| {
                // Either node flavor identifies the GPU; drivers differ in
                // which one they report.
                let matches = |queried: Result<Option<DrmNode>, _>| {
                    queried
                        .ok()
                        .flatten()
                        .is_some_and(|queried| queried.dev_id() == node.dev_id())
                };
                matches(phd.primary_node()) || matches(phd.render_node())
            })
            .ok_or_else(|| RenderInitError::NoVulkanDevice {
                node: node.to_string(),
            })?;

        tracing::info!(
            device = physical_device.name(),
            %node,
            "creating Vulkan allocator"
        );

        // COLOR_ATTACHMENT: the GLES context renders into these buffers after
        // importing them; the display controller scans them out.
        VulkanAllocator::new(&physical_device, ImageUsageFlags::COLOR_ATTACHMENT)
            .map_err(|err| RenderInitError::VulkanAllocator(err.to_string()))
    }
}

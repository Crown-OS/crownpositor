//! Framebuffers for the handful of buffers a game cycles through, so each
//! flip is one ioctl instead of an `ADDFB2` as well.

use std::rc::Rc;

use smithay::{
    backend::{
        allocator::gbm::GbmDevice,
        drm::{
            DrmDeviceFd,
            gbm::{GbmFramebuffer, framebuffer_from_wayland_buffer},
        },
    },
    reexports::wayland_server::{Resource, Weak, protocol::wl_buffer::WlBuffer},
};

use crate::backend::kms::tearing::commit::FlipError;

/// A swapchain rarely has more images than this; an older entry is only
/// dropped from the cache, never from a flip still showing it.
const CAPACITY: usize = 4;

/// Shared with the flips that show it: removing a framebuffer the plane is
/// scanning out would blank the output.
pub type SharedFramebuffer = Rc<GbmFramebuffer>;

#[derive(Debug, Default)]
pub struct FramebufferCache {
    entries: Vec<(Weak<WlBuffer>, SharedFramebuffer)>,
}

impl FramebufferCache {
    pub fn get_or_add(
        &mut self,
        drm: &DrmDeviceFd,
        gbm: &GbmDevice<DrmDeviceFd>,
        buffer: &WlBuffer,
    ) -> Result<SharedFramebuffer, FlipError> {
        self.entries.retain(|(held, _)| held.is_alive());
        if let Some((_, framebuffer)) = self
            .entries
            .iter()
            .find(|(held, _)| held.id() == buffer.id())
        {
            return Ok(Rc::clone(framebuffer));
        }

        // Opaque fallback on, as `DrmCompositor` does for the primary plane,
        // so the framebuffer lands on the format it already put on screen.
        let framebuffer = framebuffer_from_wayland_buffer(drm, gbm, buffer, true)
            .ok()
            .flatten()
            .map(Rc::new)
            .ok_or(FlipError::NoFramebuffer)?;
        if self.entries.len() == CAPACITY {
            self.entries.remove(0);
        }
        self.entries
            .push((buffer.downgrade(), Rc::clone(&framebuffer)));
        Ok(framebuffer)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

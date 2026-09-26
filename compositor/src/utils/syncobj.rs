//! A DRM timeline syncobj a client handed over, and the three things done
//! with it: signal a point from the CPU, and get an eventfd that becomes
//! readable once a point is signalled.
//!
//! Smithay's own `DrmTimeline` cannot be built into points outside its
//! `linux-drm-syncobj` implementation at this revision, so this is the same
//! handful of ioctls through the `drm` crate directly.

use std::{
    io,
    os::fd::{BorrowedFd, OwnedFd},
};

use smithay::{
    backend::drm::DrmDeviceFd,
    reexports::{
        drm::control::{Device as ControlDevice, syncobj},
        rustix::event::{EventfdFlags, eventfd},
    },
};

pub struct Timeline {
    device: DrmDeviceFd,
    handle: syncobj::Handle,
}

impl Timeline {
    pub fn import(device: &DrmDeviceFd, fd: BorrowedFd<'_>) -> io::Result<Self> {
        let handle = device.fd_to_syncobj(fd, false)?;
        Ok(Self {
            device: device.clone(),
            handle,
        })
    }

    pub fn signal(&self, point: u64) -> io::Result<()> {
        self.device
            .syncobj_timeline_signal(&[self.handle], &[point])
    }

    /// Readable once `point` is signalled. Non-blocking, so a calloop source
    /// can drain it.
    pub fn signalled_eventfd(&self, point: u64) -> io::Result<OwnedFd> {
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        let borrowed = std::os::fd::AsFd::as_fd(&fd);
        self.device
            .syncobj_eventfd(self.handle, point, borrowed, false)?;
        Ok(fd)
    }
}

impl Drop for Timeline {
    fn drop(&mut self) {
        if let Err(err) = self.device.destroy_syncobj(self.handle) {
            tracing::debug!(%err, "failed to destroy an imported syncobj");
        }
    }
}

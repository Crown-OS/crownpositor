//! The async commit itself: the primary plane's framebuffer, and nothing else.

use std::io;

use smithay::{
    backend::drm::DrmSurface,
    reexports::{
        drm::control::{
            AtomicCommitFlags, Device as ControlDevice, atomic::AtomicModeReq, framebuffer,
            property,
        },
        rustix::io::Errno,
    },
};

use crate::backend::kms::tearing::caps::AsyncCaps;

#[derive(Debug, thiserror::Error)]
pub enum FlipError {
    /// The driver would not flip this one asynchronously; a vsync commit of
    /// the same frame will do.
    #[error("the driver refused the async flip")]
    Rejected,
    #[error("the buffer cannot be made a framebuffer")]
    NoFramebuffer,
    #[error("async flip failed: {0}")]
    Io(#[from] io::Error),
}

pub fn flip_async(
    surface: &DrmSurface,
    caps: &AsyncCaps,
    framebuffer: framebuffer::Handle,
) -> Result<(), FlipError> {
    let mut request = AtomicModeReq::new();
    request.add_property(
        caps.plane,
        caps.fb_id,
        property::Value::Framebuffer(Some(framebuffer)),
    );
    let flags = AtomicCommitFlags::PAGE_FLIP_ASYNC
        | AtomicCommitFlags::PAGE_FLIP_EVENT
        | AtomicCommitFlags::NONBLOCK;
    surface.atomic_commit(flags, request).map_err(classify)
}

fn classify(err: io::Error) -> FlipError {
    match Errno::from_io_error(&err) {
        Some(Errno::INVAL | Errno::BUSY) => FlipError::Rejected,
        _ => FlipError::Io(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_and_busy_mean_try_again_with_vsync() {
        let invalid = io::Error::from_raw_os_error(Errno::INVAL.raw_os_error());
        let busy = io::Error::from_raw_os_error(Errno::BUSY.raw_os_error());
        assert!(matches!(classify(invalid), FlipError::Rejected));
        assert!(matches!(classify(busy), FlipError::Rejected));
    }

    #[test]
    fn other_failures_are_reported() {
        let denied = io::Error::from_raw_os_error(Errno::ACCESS.raw_os_error());
        assert!(matches!(classify(denied), FlipError::Io(_)));
    }
}

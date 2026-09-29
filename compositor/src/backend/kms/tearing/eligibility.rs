//! Whether a frame may skip vblank.
//!
//! An async commit may change nothing but the primary plane's framebuffer,
//! and the new framebuffer must match the old one's size, format and
//! modifier. Everything else goes through `DrmCompositor` with vsync.

use smithay::{
    backend::allocator::{Buffer as _, Format},
    reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    utils::{Buffer as BufferCoords, Size},
    wayland::dmabuf::get_dmabuf,
};

/// What the kernel compares between the framebuffer on screen and the one an
/// async flip replaces it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferLayout {
    size: Size<i32, BufferCoords>,
    format: Format,
}

impl BufferLayout {
    pub fn of(buffer: &WlBuffer) -> Option<Self> {
        let dmabuf = get_dmabuf(buffer).ok()?;
        Some(Self {
            size: dmabuf.size(),
            format: dmabuf.format(),
        })
    }
}

/// How the plane assignment laid out one frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameShape {
    /// The window that asked to tear is on the primary plane by itself.
    pub game_on_primary: bool,
    /// A cursor or overlay plane is in use, which an async commit cannot
    /// update.
    pub other_planes_used: bool,
    /// A CRTC property change is waiting, which only a vsync commit carries.
    pub commit_pending: bool,
}

impl FrameShape {
    /// Whether what this frame put on screen was the game buffer alone, the
    /// only state an async flip may continue from.
    pub fn is_bare_game(&self) -> bool {
        self.game_on_primary && !self.other_planes_used
    }
}

/// `on_screen` is the layout of the framebuffer the primary plane shows now,
/// when that is a bare game buffer.
pub fn may_flip_async(
    frame: &FrameShape,
    on_screen: Option<BufferLayout>,
    next: Option<BufferLayout>,
    flip_in_flight: bool,
) -> bool {
    frame.is_bare_game()
        && !frame.commit_pending
        && !flip_in_flight
        && on_screen.is_some()
        && on_screen == next
}

#[cfg(test)]
mod tests {
    use smithay::backend::allocator::{Fourcc, Modifier};

    use super::*;

    fn layout(fourcc: Fourcc) -> Option<BufferLayout> {
        Some(BufferLayout {
            size: Size::from((1920, 1200)),
            format: Format {
                code: fourcc,
                modifier: Modifier::Linear,
            },
        })
    }

    fn bare() -> FrameShape {
        FrameShape {
            game_on_primary: true,
            other_planes_used: false,
            commit_pending: false,
        }
    }

    #[test]
    fn a_bare_game_frame_of_the_same_layout_flips() {
        let same = layout(Fourcc::Xrgb8888);
        assert!(may_flip_async(&bare(), same, same, false));
    }

    #[test]
    fn a_layout_change_waits_for_vblank() {
        assert!(!may_flip_async(
            &bare(),
            layout(Fourcc::Xrgb8888),
            layout(Fourcc::Argb8888),
            false
        ));
    }

    #[test]
    fn nothing_flips_without_a_game_buffer_on_screen() {
        let same = layout(Fourcc::Xrgb8888);
        assert!(!may_flip_async(&bare(), None, same, false));
    }

    #[test]
    fn a_visible_cursor_or_overlay_forces_vsync() {
        let same = layout(Fourcc::Xrgb8888);
        let cursor = FrameShape {
            other_planes_used: true,
            ..bare()
        };
        assert!(!may_flip_async(&cursor, same, same, false));
    }

    #[test]
    fn a_pending_property_change_forces_vsync() {
        let same = layout(Fourcc::Xrgb8888);
        let pending = FrameShape {
            commit_pending: true,
            ..bare()
        };
        assert!(!may_flip_async(&pending, same, same, false));
    }

    #[test]
    fn one_flip_at_a_time() {
        let same = layout(Fourcc::Xrgb8888);
        assert!(!may_flip_async(&bare(), same, same, true));
    }
}

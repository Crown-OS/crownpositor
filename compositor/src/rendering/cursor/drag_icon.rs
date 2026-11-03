//! The picture a client's drag carries under the pointer, drawn by the
//! compositor so it follows the pointer over every surface.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
    wayland::compositor::{SurfaceAttributes, with_states},
};

/// A drag's icon surface, its origin `offset` from the pointer's hot point.
pub struct DragIcon {
    pub surface: WlSurface,
    pub offset: Point<i32, Logical>,
}

impl DragIcon {
    pub fn new(surface: WlSurface) -> Self {
        Self {
            surface,
            offset: Point::default(),
        }
    }

    /// Moves the icon by what its latest commit asked for, through
    /// `wl_surface.offset` or an attach offset.
    pub fn commit(&mut self) {
        let delta = with_states(&self.surface, |states| {
            states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer_delta
                .take()
        });
        if let Some(delta) = delta {
            self.offset += delta;
        }
    }
}

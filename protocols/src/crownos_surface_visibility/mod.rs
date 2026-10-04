//! Server-side `crownos_surface_visibility_v1`.
//!
//! Bookkeeping only. The compositor works out each observed surface's
//! [`Visibility`] once per scene update and hands it to
//! [`SurfaceVisibilityState::refresh`], which tells each client only what
//! changed since it was last told. Per-object state lives here rather than in
//! the resources' user data, so none of it needs a lock.

mod dispatch;
mod visibility;

use crownos_protocols::surface_visibility::v1::server::{
    crownos_surface_visibility_manager_v1::CrownosSurfaceVisibilityManagerV1,
    crownos_surface_visibility_v1::CrownosSurfaceVisibilityV1,
};
use wayland_server::{DisplayHandle, Resource, backend::GlobalId, protocol::wl_surface::WlSurface};

pub use dispatch::SurfaceVisibilityDispatch;
pub use visibility::{ThumbnailScale, Visibility, VisibilityState};

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`SurfaceVisibilityState`].
pub trait SurfaceVisibilityHandler {
    fn surface_visibility_state(&mut self) -> &mut SurfaceVisibilityState;
}

struct Observer {
    resource: CrownosSurfaceVisibilityV1,
    surface: WlSurface,
    reported: Option<Visibility>,
}

impl Observer {
    fn report(&mut self, current: Visibility) {
        let Some(changes) = current.changes_since(self.reported) else {
            return;
        };
        self.reported = Some(current);
        if let Some(scale) = changes.thumbnail_scale {
            self.resource.thumbnail_scale(scale.as_f64());
        }
        if let Some(state) = changes.state {
            self.resource.state(state);
        }
        self.resource.done();
    }
}

/// Delegate type for the `crownos_surface_visibility_manager_v1` global.
pub struct SurfaceVisibilityState {
    global: GlobalId,
    observers: Vec<Observer>,
}

impl SurfaceVisibilityState {
    pub fn new<D: SurfaceVisibilityDispatch>(display: &DisplayHandle) -> Self {
        let global = display.create_global::<D, CrownosSurfaceVisibilityManagerV1, _>(VERSION, ());
        Self {
            global,
            observers: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Sends every observer whatever changed. A surface that is gone is
    /// reported hidden once and then left alone.
    pub fn refresh(&mut self, mut visibility_of: impl FnMut(&WlSurface) -> Visibility) {
        for observer in &mut self.observers {
            let current = if observer.surface.is_alive() {
                visibility_of(&observer.surface)
            } else {
                Visibility::HIDDEN
            };
            observer.report(current);
        }
    }

    fn observe(&mut self, resource: CrownosSurfaceVisibilityV1, surface: WlSurface) {
        self.observers.push(Observer {
            resource,
            surface,
            reported: None,
        });
    }

    fn forget(&mut self, resource: &CrownosSurfaceVisibilityV1) {
        self.observers
            .retain(|observer| observer.resource != *resource);
    }
}

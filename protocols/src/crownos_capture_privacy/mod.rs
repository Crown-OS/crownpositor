//! Server-side `crownos_capture_privacy_v1`: surfaces their applications
//! keep out of captures.

mod dispatch;

use std::collections::HashSet;

use crownos_protocols::capture_privacy::v1::server::crownos_capture_privacy_manager_v1::CrownosCapturePrivacyManagerV1;
use wayland_server::{
    DisplayHandle, Resource, Weak,
    backend::{GlobalId, ObjectId},
    protocol::wl_surface::WlSurface,
};

pub use dispatch::CapturePrivacyDispatch;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`CapturePrivacyState`].
pub trait CapturePrivacyHandler {
    fn capture_privacy_state(&mut self) -> &mut CapturePrivacyState;
    /// A surface's exclusion changed, so captures showing it need another look.
    fn surface_privacy_changed(&mut self, surface: &WlSurface);
}

/// User data of a `crownos_surface_privacy_v1`.
#[derive(Debug)]
pub struct PrivacyData {
    surface: Weak<WlSurface>,
}

/// Delegate type for the `crownos_capture_privacy_manager_v1` global.
pub struct CapturePrivacyState {
    global: GlobalId,
    /// Surfaces with a privacy object, excluded or not: one per surface.
    claimed: HashSet<ObjectId>,
    excluded: HashSet<ObjectId>,
}

impl CapturePrivacyState {
    pub fn new<D: CapturePrivacyDispatch>(display: &DisplayHandle) -> Self {
        let global = display.create_global::<D, CrownosCapturePrivacyManagerV1, _>(VERSION, ());
        Self {
            global,
            claimed: HashSet::new(),
            excluded: HashSet::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Whether the application asked that this surface stay out of captures.
    pub fn is_excluded(&self, surface: &WlSurface) -> bool {
        self.excluded.contains(&surface.id())
    }

    fn claim(&mut self, surface: &WlSurface) -> bool {
        self.claimed.insert(surface.id())
    }

    /// Returns whether the exclusion changed.
    fn set_excluded(&mut self, surface: &WlSurface, excluded: bool) -> bool {
        if excluded {
            self.excluded.insert(surface.id())
        } else {
            self.excluded.remove(&surface.id())
        }
    }

    fn release(&mut self, surface: &WlSurface) -> bool {
        self.claimed.remove(&surface.id());
        self.excluded.remove(&surface.id())
    }
}

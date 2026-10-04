//! Server-side `crownos_surface_animation_v1`.
//!
//! A client hands the compositor a subsurface and a spring per property; the
//! compositor samples [`motion::Animation`] on every output frame, so the app
//! and the compositor evaluate the same closed-form curve from the same
//! CLOCK_MONOTONIC start time and the app does not render the motion itself.
//!
//! Requests are double-buffered through the surface's cached state and applied
//! by a post-commit hook. The sampled result lives in the surface's
//! `data_map`, where the renderer reads it as a [`SurfaceTransform`] while it
//! walks the surface tree.

mod animations;
mod dispatch;
mod pending;
mod property;
mod surface_motion;
mod track;
mod tracks;
mod transform;

pub use animations::SurfaceAnimations;
pub use dispatch::SurfaceAnimationDispatch;
pub use surface_motion::surface_transform;
pub use transform::SurfaceTransform;

use crownos_protocols::surface_animation::v1::server::crownos_surface_animation_manager_v1::CrownosSurfaceAnimationManagerV1;
use wayland_server::{
    DisplayHandle, Resource, Weak, backend::GlobalId, protocol::wl_surface::WlSurface,
};

/// What the compositor has to provide for this protocol to be delegated to
/// [`SurfaceAnimationState`].
pub trait SurfaceAnimationHandler {
    fn surface_animations(&mut self) -> &mut SurfaceAnimations;
}

#[derive(Debug)]
pub struct SurfaceAnimationState {
    global: GlobalId,
}

impl SurfaceAnimationState {
    pub fn new<D: SurfaceAnimationDispatch>(display: &DisplayHandle) -> Self {
        Self {
            global: display.create_global::<D, CrownosSurfaceAnimationManagerV1, _>(1, ()),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

/// User data of a `crownos_animated_surface_v1`. Holds the surface weakly, so
/// the object goes inert when its surface dies.
#[derive(Debug)]
pub struct AnimatedSurfaceData {
    surface: Weak<WlSurface>,
}

impl AnimatedSurfaceData {
    fn new(surface: &WlSurface) -> Self {
        Self {
            surface: surface.downgrade(),
        }
    }

    fn wl_surface(&self) -> Option<WlSurface> {
        self.surface.upgrade().ok()
    }
}

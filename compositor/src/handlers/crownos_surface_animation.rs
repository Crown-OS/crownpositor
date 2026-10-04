//! Delegation glue for `crownos_surface_animation_v1`. The logic lives in
//! [`protocols::crownos_surface_animation`].

use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crownos_protocols::surface_animation::v1::server::{
    crownos_animated_surface_v1::CrownosAnimatedSurfaceV1,
    crownos_surface_animation_manager_v1::CrownosSurfaceAnimationManagerV1,
};
use protocols::crownos_surface_animation::{
    AnimatedSurfaceData, SurfaceAnimationHandler, SurfaceAnimationState, SurfaceAnimations,
};

use crate::state::State;

impl SurfaceAnimationHandler for State {
    fn surface_animations(&mut self) -> &mut SurfaceAnimations {
        &mut self.shell.surface_animations
    }
}

delegate_global_dispatch!(State: [CrownosSurfaceAnimationManagerV1: ()] => SurfaceAnimationState);
delegate_dispatch!(State: [CrownosSurfaceAnimationManagerV1: ()] => SurfaceAnimationState);
delegate_dispatch!(State: [CrownosAnimatedSurfaceV1: AnimatedSurfaceData] => SurfaceAnimationState);

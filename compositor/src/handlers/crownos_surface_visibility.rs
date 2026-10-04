//! Delegation glue for `crownos_surface_visibility_v1`: the protocol lives in
//! [`protocols::crownos_surface_visibility`], the answers in
//! [`crate::shell::visibility`].

use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crownos_protocols::surface_visibility::v1::server::{
    crownos_surface_visibility_manager_v1::CrownosSurfaceVisibilityManagerV1,
    crownos_surface_visibility_v1::CrownosSurfaceVisibilityV1,
};
use protocols::crownos_surface_visibility::{SurfaceVisibilityHandler, SurfaceVisibilityState};

use crate::state::State;

impl SurfaceVisibilityHandler for State {
    fn surface_visibility_state(&mut self) -> &mut SurfaceVisibilityState {
        &mut self.wayland.surface_visibility_state
    }
}

delegate_global_dispatch!(State: [CrownosSurfaceVisibilityManagerV1: ()] => SurfaceVisibilityState);
delegate_dispatch!(State: [CrownosSurfaceVisibilityManagerV1: ()] => SurfaceVisibilityState);
delegate_dispatch!(State: [CrownosSurfaceVisibilityV1: ()] => SurfaceVisibilityState);

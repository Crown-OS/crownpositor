//! Delegation glue for `crownos_background_effects`.
//!
//! All the logic lives in [`protocols::crownos_background_effects`]; this file
//! only routes the four interfaces to it and hands it the state, the same way
//! smithay's `delegate_*!` macros do for the protocols it ships.

use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crownos_protocols::background_effects::v1::server::{
    crownos_background_effect_manager_v1::CrownosBackgroundEffectManagerV1,
    crownos_background_effect_surface_v1::CrownosBackgroundEffectSurfaceV1,
    crownos_shape_manager_v1::CrownosShapeManagerV1, crownos_shape_v1::CrownosShapeV1,
};
use protocols::crownos_background_effects::{
    BackgroundEffectsHandler, BackgroundEffectsState, EffectSurfaceData, GlobalData, ShapeData,
};

use crate::state::State;

impl BackgroundEffectsHandler for State {
    fn background_effects_state(&mut self) -> &mut BackgroundEffectsState {
        &mut self.wayland.crownos_background_effects_state
    }
}

delegate_global_dispatch!(State: [CrownosShapeManagerV1: GlobalData] => BackgroundEffectsState);
delegate_dispatch!(State: [CrownosShapeManagerV1: GlobalData] => BackgroundEffectsState);
delegate_dispatch!(State: [CrownosShapeV1: ShapeData] => BackgroundEffectsState);

delegate_global_dispatch!(State: [CrownosBackgroundEffectManagerV1: GlobalData] => BackgroundEffectsState);
delegate_dispatch!(State: [CrownosBackgroundEffectManagerV1: GlobalData] => BackgroundEffectsState);
delegate_dispatch!(State: [CrownosBackgroundEffectSurfaceV1: EffectSurfaceData] => BackgroundEffectsState);

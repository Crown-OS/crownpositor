//! Delegation glue for `ext-background-effect-v1`.
//!
//! All the logic lives in [`protocols::background_effect`]; this file only
//! routes the interfaces to it and hands it the state, the same way smithay's
//! `delegate_*!` macros do for the protocols it ships.

use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use protocols::background_effect::{
    BackgroundEffectGlobalData, BackgroundEffectHandler, BackgroundEffectState,
    BackgroundEffectSurfaceData,
};

use crate::state::State;

impl BackgroundEffectHandler for State {
    fn background_effect_state(&mut self) -> &mut BackgroundEffectState {
        &mut self.wayland.background_effect_state
    }
}

delegate_global_dispatch!(State: [ExtBackgroundEffectManagerV1: BackgroundEffectGlobalData] => BackgroundEffectState);
delegate_dispatch!(State: [ExtBackgroundEffectManagerV1: BackgroundEffectGlobalData] => BackgroundEffectState);
delegate_dispatch!(State: [ExtBackgroundEffectSurfaceV1: BackgroundEffectSurfaceData] => BackgroundEffectState);

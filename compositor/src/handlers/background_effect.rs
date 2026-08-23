//! Delegation glue for `ext-background-effect-v1`.
//!
//! All the logic lives in [`protocols::background_effect`]; this file only
//! routes the interfaces to it, the same way smithay's `delegate_*!` macros
//! do for the protocols it ships.

use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{
    protocols::background_effect::{
        BackgroundEffectGlobalData, BackgroundEffectState, BackgroundEffectSurfaceData,
    },
    state::State,
};

delegate_global_dispatch!(State: [ExtBackgroundEffectManagerV1: BackgroundEffectGlobalData] => BackgroundEffectState);
delegate_dispatch!(State: [ExtBackgroundEffectManagerV1: BackgroundEffectGlobalData] => BackgroundEffectState);
delegate_dispatch!(State: [ExtBackgroundEffectSurfaceV1: BackgroundEffectSurfaceData] => BackgroundEffectState);

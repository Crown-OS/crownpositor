//! Delegation glue for `crownos_context_v1`: the protocol lives in
//! [`protocols::crownos_context`], the window records in
//! [`crate::state::windows`].

use std::time::Duration;

use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::CrownosContextManagerV1,
    crownos_context_window_v1::CrownosContextWindowV1,
};
use protocols::crownos_context::{
    ContextGlobalData, ContextHandler, ContextState, FlagReason, PauseOrigin, WindowKey,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{state::State, utils::id::WindowId};

const AUDIT: &str = "crownpositor::audit::context";
/// The longest an engine flag is honoured in one request.
const MAX_FLAG: Duration = Duration::from_secs(600);

impl ContextHandler for State {
    fn context_state(&mut self) -> &mut ContextState {
        &mut self.wayland.context_state
    }

    fn set_context_paused(&mut self, paused: bool) {
        tracing::info!(target: AUDIT, paused, "engine set pause");
        self.wayland
            .context_state
            .set_paused(paused, PauseOrigin::Engine);
    }

    fn flag_window_sensitive(&mut self, key: WindowKey, reason: FlagReason, duration_ms: u32) {
        let id = WindowId::from_raw(key);
        let duration = Duration::from_millis(u64::from(duration_ms)).min(MAX_FLAG);
        tracing::info!(target: AUDIT, %id, ?reason, ?duration, "engine flagged a window");
        self.windows.flag(id, duration);
    }

    fn context_engine_gone(&mut self) {
        tracing::info!(target: AUDIT, "engine disconnected");
    }
}

delegate_global_dispatch!(State: [CrownosContextManagerV1: ContextGlobalData] => ContextState);
delegate_dispatch!(State: [CrownosContextManagerV1: ()] => ContextState);
delegate_dispatch!(State: [CrownosContextWindowV1: WindowKey] => ContextState);

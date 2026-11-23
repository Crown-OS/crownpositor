//! Delegation glue for `crownos_context_v1`: the protocol lives in
//! [`protocols::crownos_context`], the window records in
//! [`crate::state::windows`].

use std::time::Duration;

use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::CrownosContextManagerV1,
    crownos_context_window_v1::CrownosContextWindowV1,
};
use protocols::crownos_context::{
    BindDecision, ContextGlobalData, ContextHandler, ContextState, FlagReason, PauseOrigin,
    WindowKey,
};
use wayland_server::{Client, backend::ClientId, delegate_dispatch, delegate_global_dispatch};

use crate::{
    state::{State, context_access::AUDIT},
    utils::id::WindowId,
};

/// The longest an engine flag is honoured in one request.
const MAX_FLAG: Duration = Duration::from_secs(600);

impl ContextHandler for State {
    fn context_state(&mut self) -> &mut ContextState {
        &mut self.wayland.context_state
    }

    fn context_bind_decision(&mut self, client: &Client) -> BindDecision {
        self.context_bind_decision_for(client)
    }

    fn set_context_paused(&mut self, paused: bool, origin: PauseOrigin) {
        tracing::info!(target: AUDIT, paused, ?origin, "observation paused for every client");
        self.wayland.context_state.set_paused(paused, origin);
    }

    fn flag_window_sensitive(&mut self, key: WindowKey, reason: FlagReason, duration_ms: u32) {
        let id = WindowId::from_raw(key);
        let duration = Duration::from_millis(u64::from(duration_ms)).min(MAX_FLAG);
        tracing::info!(target: AUDIT, %id, ?reason, ?duration, "a client flagged a window");
        self.windows.flag(id, duration);
    }

    fn context_client_gone(&mut self, client: Option<ClientId>) {
        State::context_client_gone(self, client);
    }
}

delegate_global_dispatch!(State: [CrownosContextManagerV1: ContextGlobalData] => ContextState);
delegate_dispatch!(State: [CrownosContextManagerV1: ()] => ContextState);
delegate_dispatch!(State: [CrownosContextWindowV1: WindowKey] => ContextState);

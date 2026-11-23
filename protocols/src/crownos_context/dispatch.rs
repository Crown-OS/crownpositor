//! Request routing for `crownos_context_v1`.

use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::{self, CrownosContextManagerV1},
    crownos_context_window_v1::{self, CrownosContextWindowV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, WEnum, backend::ClientId,
};

use super::{ContextGlobalData, ContextHandler, ContextState, PauseOrigin, Scope, WindowKey};

/// The bounds every impl in this module shares, named once.
pub trait ContextDispatch:
    GlobalDispatch<CrownosContextManagerV1, ContextGlobalData>
    + Dispatch<CrownosContextManagerV1, ()>
    + Dispatch<CrownosContextWindowV1, WindowKey>
    + ContextHandler
    + 'static
{
}

impl<D> ContextDispatch for D where
    D: GlobalDispatch<CrownosContextManagerV1, ContextGlobalData>
        + Dispatch<CrownosContextManagerV1, ()>
        + Dispatch<CrownosContextWindowV1, WindowKey>
        + ContextHandler
        + 'static
{
}

impl<D: ContextDispatch> GlobalDispatch<CrownosContextManagerV1, ContextGlobalData, D>
    for ContextState
{
    fn bind(
        state: &mut D,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<CrownosContextManagerV1>,
        _global_data: &ContextGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());
        let decision = state.context_bind_decision(client);
        state.context_state().bind::<D>(handle, manager, decision);
    }

    fn can_view(client: Client, global_data: &ContextGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: ContextDispatch> Dispatch<CrownosContextManagerV1, (), D> for ContextState {
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosContextManagerV1,
        request: crownos_context_manager_v1::Request,
        _data: &(),
        dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_context_manager_v1::Request::SetPaused { paused } = request else {
            return;
        };
        let Some(bound) = state
            .context_state()
            .bound_of(manager)
            .filter(|bound| bound.is_live())
        else {
            return;
        };
        if !bound.pauses_everyone() {
            state
                .context_state()
                .set_client_paused::<D>(dhandle, manager, paused != 0);
            return;
        }
        let origin = if bound.scope().contains(Scope::Engine) {
            PauseOrigin::Engine
        } else {
            PauseOrigin::Client
        };
        state.set_context_paused(paused != 0, origin);
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &CrownosContextManagerV1, _data: &()) {
        if let Some(client) = state.context_state().unbind(resource) {
            state.context_client_gone(client);
        }
    }
}

impl<D: ContextDispatch> Dispatch<CrownosContextWindowV1, WindowKey, D> for ContextState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosContextWindowV1,
        request: crownos_context_window_v1::Request,
        key: &WindowKey,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_context_window_v1::Request::FlagSensitive {
            reason: WEnum::Value(reason),
            duration_ms,
        } = request
        else {
            return;
        };
        let context = state.context_state();
        if context.knows(*key) && context.may_flag(resource, *key) {
            state.flag_window_sensitive(*key, reason, duration_ms);
        }
    }
}

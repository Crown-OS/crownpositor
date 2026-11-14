//! Request routing for `crownos_context_v1`.

use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::{self, CrownosContextManagerV1},
    crownos_context_window_v1::{self, CrownosContextWindowV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, WEnum, backend::ClientId,
};

use super::{ContextGlobalData, ContextHandler, ContextState, WindowKey};

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
        _client: &Client,
        resource: New<CrownosContextManagerV1>,
        _global_data: &ContextGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());
        if !state.context_state().bind::<D>(handle, manager) {
            tracing::warn!("a second client bound crownos_context_manager_v1 and was left inert");
        }
    }

    fn can_view(client: Client, global_data: &ContextGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: ContextDispatch> Dispatch<CrownosContextManagerV1, (), D> for ContextState {
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &CrownosContextManagerV1,
        request: crownos_context_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let crownos_context_manager_v1::Request::SetPaused { paused } = request {
            state.set_context_paused(paused != 0);
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &CrownosContextManagerV1, _data: &()) {
        if state.context_state().unbind(resource) {
            state.context_engine_gone();
        }
    }
}

impl<D: ContextDispatch> Dispatch<CrownosContextWindowV1, WindowKey, D> for ContextState {
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &CrownosContextWindowV1,
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
        if state.context_state().knows(*key) {
            state.flag_window_sensitive(*key, reason, duration_ms);
        }
    }
}

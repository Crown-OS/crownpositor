//! Request routing for `crownos_plugin_host_v1`.

use crownos_protocols::plugin_host::v1::server::crownos_plugin_host_v1::{
    self, ActionResult, CrownosPluginHostV1,
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId,
};

use super::{
    ASSET_BYTES, ActionOutcome, PayloadError, PluginHostGlobalData, PluginHostHandler,
    PluginHostState, RequestError, SCENE_BYTES, SLOT_BYTES, payload,
};

pub trait PluginHostDispatch:
    GlobalDispatch<CrownosPluginHostV1, PluginHostGlobalData>
    + Dispatch<CrownosPluginHostV1, ()>
    + PluginHostHandler
    + 'static
{
}

impl<D> PluginHostDispatch for D where
    D: GlobalDispatch<CrownosPluginHostV1, PluginHostGlobalData>
        + Dispatch<CrownosPluginHostV1, ()>
        + PluginHostHandler
        + 'static
{
}

impl<D: PluginHostDispatch> GlobalDispatch<CrownosPluginHostV1, PluginHostGlobalData, D>
    for PluginHostState
{
    fn bind(
        state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosPluginHostV1>,
        _global_data: &PluginHostGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let host = data_init.init(resource, ());
        let plugin_host = state.plugin_host_state();
        if plugin_host.is_bound() {
            host.post_error(
                crownos_plugin_host_v1::Error::AlreadyBound,
                "crownos_plugin_host_v1 is already bound",
            );
            return;
        }
        host.lock_state(u32::from(plugin_host.locked));
        plugin_host.watching = 0;
        plugin_host.bound = Some(host);
    }

    fn can_view(client: Client, global_data: &PluginHostGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: PluginHostDispatch> Dispatch<CrownosPluginHostV1, (), D> for PluginHostState {
    fn request(
        state: &mut D,
        _client: &Client,
        host: &CrownosPluginHostV1,
        request: crownos_plugin_host_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if state.plugin_host_state().bound.as_ref() != Some(host) {
            return;
        }
        use crownos_plugin_host_v1::Request;
        let outcome = match request {
            Request::Destroy => Ok(()),
            Request::Watch { events } => {
                state.plugin_host_state().watching = match events {
                    WEnum::Value(events) => events.bits(),
                    WEnum::Unknown(bits) => bits,
                };
                Ok(())
            }
            Request::RunAction { serial, action } => {
                let result = match state.run_plugin_action(&action) {
                    ActionOutcome::Done => ActionResult::Done,
                    ActionOutcome::Invalid => ActionResult::Invalid,
                    ActionOutcome::Refused => ActionResult::Refused,
                };
                host.action_done(serial, result);
                Ok(())
            }
            Request::SetLockScene { fd, size } => payload::read(&fd, size, SCENE_BYTES)
                .map_err(RequestError::from)
                .and_then(|bytes| state.set_lock_scene(bytes)),
            Request::SetLockSlot { id, fd, size } => payload::read(&fd, size, SLOT_BYTES)
                .map_err(RequestError::from)
                .and_then(|bytes| state.set_lock_slot(&id, bytes)),
            Request::PutLockAsset { id, fd, size } => payload::read(&fd, size, ASSET_BYTES)
                .map_err(RequestError::from)
                .and_then(|bytes| state.put_lock_asset(&id, bytes)),
            _ => Ok(()),
        };
        match outcome {
            Ok(()) => {}
            Err(RequestError::TooLarge) => {
                host.post_error(crownos_plugin_host_v1::Error::TooLarge, "payload too large");
            }
            Err(RequestError::Invalid(reason)) => {
                host.post_error(crownos_plugin_host_v1::Error::InvalidPayload, reason);
            }
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, host: &CrownosPluginHostV1, _data: &()) {
        let plugin_host = state.plugin_host_state();
        if plugin_host
            .bound
            .as_ref()
            .is_some_and(|bound| bound.id() == host.id())
        {
            plugin_host.bound = None;
            plugin_host.watching = 0;
            state.plugin_host_gone();
        }
    }
}

impl From<PayloadError> for RequestError {
    fn from(error: PayloadError) -> Self {
        match error {
            PayloadError::TooLarge { .. } => Self::TooLarge,
            PayloadError::Unreadable => Self::Invalid(error.to_string()),
        }
    }
}

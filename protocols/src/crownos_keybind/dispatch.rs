//! Request routing for `crownos_keybind_v1`.

use crownos_protocols::keybind::v1::server::{
    crownos_keybind_manager_v1::{self, CrownosKeybindManagerV1, Error},
    crownos_keybind_v1::{self, CrownosKeybindV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
};

use super::{KeybindHandler, KeybindRegistration, KeybindState};

/// The bounds every impl in this module shares, named once.
pub trait KeybindDispatch:
    GlobalDispatch<CrownosKeybindManagerV1, ()>
    + Dispatch<CrownosKeybindManagerV1, ()>
    + Dispatch<CrownosKeybindV1, ()>
    + KeybindHandler
    + 'static
{
}

impl<D> KeybindDispatch for D where
    D: GlobalDispatch<CrownosKeybindManagerV1, ()>
        + Dispatch<CrownosKeybindManagerV1, ()>
        + Dispatch<CrownosKeybindV1, ()>
        + KeybindHandler
        + 'static
{
}

impl<D: KeybindDispatch> GlobalDispatch<CrownosKeybindManagerV1, (), D> for KeybindState {
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosKeybindManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D: KeybindDispatch> Dispatch<CrownosKeybindManagerV1, (), D> for KeybindState {
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosKeybindManagerV1,
        request: crownos_keybind_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_keybind_manager_v1::Request::Register {
            id,
            app_id,
            name,
            description,
            preferred_keys,
        } = request
        else {
            return;
        };
        if app_id.is_empty() || name.is_empty() {
            manager.post_error(Error::InvalidIdentity, "app_id and name must not be empty");
            return;
        }
        let resource = data_init.init(id, ());
        state.keybind_state().record(KeybindRegistration::new(
            resource,
            app_id,
            name,
            description,
            preferred_keys,
        ));
        state.keybinds_changed();
    }
}

impl<D: KeybindDispatch> Dispatch<CrownosKeybindV1, (), D> for KeybindState {
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &CrownosKeybindV1,
        _request: crownos_keybind_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &CrownosKeybindV1, _data: &()) {
        if state.keybind_state().release(resource) {
            state.keybinds_changed();
        }
    }
}

//! Request routing for `crownos_window_control_v1`.

use crownos_protocols::window_control::v1::server::{
    crownos_window_control_manager_v1::{self, CrownosWindowControlManagerV1},
    crownos_window_v1::{self, CrownosWindowV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, backend::ClientId,
};

use super::{WindowControlGlobalData, WindowControlHandler, WindowControlState, WindowKey};

/// The bounds every impl in this module shares, named once.
pub trait WindowControlDispatch:
    GlobalDispatch<CrownosWindowControlManagerV1, WindowControlGlobalData>
    + Dispatch<CrownosWindowControlManagerV1, ()>
    + Dispatch<CrownosWindowV1, WindowKey>
    + WindowControlHandler
    + 'static
{
}

impl<D> WindowControlDispatch for D where
    D: GlobalDispatch<CrownosWindowControlManagerV1, WindowControlGlobalData>
        + Dispatch<CrownosWindowControlManagerV1, ()>
        + Dispatch<CrownosWindowV1, WindowKey>
        + WindowControlHandler
        + 'static
{
}

impl<D: WindowControlDispatch>
    GlobalDispatch<CrownosWindowControlManagerV1, WindowControlGlobalData, D>
    for WindowControlState
{
    fn bind(
        state: &mut D,
        handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosWindowControlManagerV1>,
        _global_data: &WindowControlGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let resource = data_init.init(resource, ());
        state.window_control_state().bind::<D>(handle, resource);
    }

    fn can_view(client: Client, global_data: &WindowControlGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: WindowControlDispatch> Dispatch<CrownosWindowControlManagerV1, (), D>
    for WindowControlState
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosWindowControlManagerV1,
        request: crownos_window_control_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let crownos_window_control_manager_v1::Request::GetAgentWorkspace = request {
            let workspace = state.agent_workspace_identifier();
            manager.agent_workspace(workspace);
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosWindowControlManagerV1,
        _data: &(),
    ) {
        state.window_control_state().unbind(resource);
    }
}

impl<D: WindowControlDispatch> Dispatch<CrownosWindowV1, WindowKey, D> for WindowControlState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosWindowV1,
        request: crownos_window_v1::Request,
        key: &WindowKey,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let control = state.window_control_state();
        if !control.knows(*key) || !control.is_live_window(resource) {
            return;
        }
        match request {
            crownos_window_v1::Request::Focus => state.focus_window(*key),
            crownos_window_v1::Request::Close => state.close_window(*key),
            crownos_window_v1::Request::MoveToWorkspace { workspace } => {
                state.move_window_to_workspace(*key, &workspace);
            }
            crownos_window_v1::Request::MoveToOutput { output } => {
                state.move_window_to_output(*key, &output);
            }
            crownos_window_v1::Request::MakeVisible => state.make_window_visible(*key),
            crownos_window_v1::Request::MakeHidden => state.make_window_hidden(*key),
            crownos_window_v1::Request::SetRectangle {
                x,
                y,
                width,
                height,
            } => state.set_window_rectangle(*key, x, y, width, height),
            crownos_window_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

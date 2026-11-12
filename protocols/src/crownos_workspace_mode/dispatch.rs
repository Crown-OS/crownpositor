//! Request routing for `crownos_workspace_mode_v1`.

use crownos_protocols::workspace_mode::v1::server::crownos_workspace_mode_manager_v1::{
    self, CrownosWorkspaceModeManagerV1, Error,
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId,
};

use super::{WorkspaceModeGlobalData, WorkspaceModeHandler, WorkspaceModeState};

/// The bounds every impl in this module shares, named once.
pub trait WorkspaceModeDispatch:
    GlobalDispatch<CrownosWorkspaceModeManagerV1, WorkspaceModeGlobalData>
    + Dispatch<CrownosWorkspaceModeManagerV1, ()>
    + WorkspaceModeHandler
    + 'static
{
}

impl<D> WorkspaceModeDispatch for D where
    D: GlobalDispatch<CrownosWorkspaceModeManagerV1, WorkspaceModeGlobalData>
        + Dispatch<CrownosWorkspaceModeManagerV1, ()>
        + WorkspaceModeHandler
        + 'static
{
}

impl<D: WorkspaceModeDispatch>
    GlobalDispatch<CrownosWorkspaceModeManagerV1, WorkspaceModeGlobalData, D>
    for WorkspaceModeState
{
    fn bind(
        state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosWorkspaceModeManagerV1>,
        _global_data: &WorkspaceModeGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());
        let protocol = state.workspace_mode_state();
        if let Some(mode) = protocol.mode {
            manager.mode(mode);
        }
        protocol.managers.push(manager);
    }

    fn can_view(client: Client, global_data: &WorkspaceModeGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: WorkspaceModeDispatch> Dispatch<CrownosWorkspaceModeManagerV1, (), D>
    for WorkspaceModeState
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosWorkspaceModeManagerV1,
        request: crownos_workspace_mode_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_workspace_mode_manager_v1::Request::SetMode { mode } = request else {
            return;
        };
        match mode {
            WEnum::Value(mode) => state.set_workspace_mode(mode),
            WEnum::Unknown(value) => manager.post_error(
                Error::InvalidMode,
                format!("{value} names no workspace mode"),
            ),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosWorkspaceModeManagerV1,
        _data: &(),
    ) {
        state
            .workspace_mode_state()
            .managers
            .retain(|manager| manager.id() != resource.id());
    }
}

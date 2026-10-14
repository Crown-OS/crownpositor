use wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientId, ObjectId},
};

use crate::ext_workspace::{ExtWorkspaceDispatch, ExtWorkspaceState, Pending};

pub struct ExtWorkspaceGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

impl ExtWorkspaceGlobalData {
    pub(super) fn new(filter: impl Fn(&Client) -> bool + Send + Sync + 'static) -> Self {
        Self {
            filter: Box::new(filter),
        }
    }
}

#[derive(Debug, Default)]
pub struct ManagerData {
    pending: Pending,
}

#[derive(Debug)]
pub struct WorkspaceData {
    pub(super) key: u64,
    pub(super) manager: ExtWorkspaceManagerV1,
}

impl<D: ExtWorkspaceDispatch> GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData, D>
    for ExtWorkspaceState
{
    fn bind(
        state: &mut D,
        display: &DisplayHandle,
        _client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        _global_data: &ExtWorkspaceGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ManagerData::default());
        state.ext_workspace_state().bind::<D>(display, manager);
    }

    fn can_view(client: Client, global_data: &ExtWorkspaceGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: ExtWorkspaceDispatch> Dispatch<ExtWorkspaceManagerV1, ManagerData, D>
    for ExtWorkspaceState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ExtWorkspaceManagerV1,
        request: ext_workspace_manager_v1::Request,
        data: &ManagerData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_workspace_manager_v1::Request::Commit => {
                for key in data.pending.take() {
                    state.activate_workspace(key);
                }
            }
            ext_workspace_manager_v1::Request::Stop => {
                state.ext_workspace_state().unbind(resource);
                resource.finished();
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtWorkspaceManagerV1,
        _data: &ManagerData,
    ) {
        state.ext_workspace_state().unbind(resource);
    }
}

impl<D: ExtWorkspaceDispatch> Dispatch<ExtWorkspaceGroupHandleV1, u64, D> for ExtWorkspaceState {
    /// `create_workspace` is never advertised; workspaces here come and go
    /// with their contents.
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ExtWorkspaceGroupHandleV1,
        _request: ext_workspace_group_handle_v1::Request,
        _data: &u64,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtWorkspaceGroupHandleV1,
        _data: &u64,
    ) {
        forget(state, Resource::id(resource));
    }
}

impl<D: ExtWorkspaceDispatch> Dispatch<ExtWorkspaceHandleV1, WorkspaceData, D>
    for ExtWorkspaceState
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ExtWorkspaceHandleV1,
        request: ext_workspace_handle_v1::Request,
        data: &WorkspaceData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let ext_workspace_handle_v1::Request::Activate = request
            && let Some(manager) = data.manager.data::<ManagerData>()
        {
            manager.pending.push(data.key);
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &ExtWorkspaceHandleV1,
        _data: &WorkspaceData,
    ) {
        forget(state, Resource::id(resource));
    }
}

fn forget<D: ExtWorkspaceDispatch>(state: &mut D, id: ObjectId) {
    state.ext_workspace_state().forget_handle(&id);
}

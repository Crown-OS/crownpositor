//! Request routing for `crownos_agent_access_v1`.

use crownos_protocols::agent_access::v1::server::{
    crownos_agent_access_manager_v1::{self, CrownosAgentAccessManagerV1},
    crownos_agent_access_v1::{self, CrownosAgentAccessV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, backend::ClientId,
};

use super::{AgentAccessGlobalData, AgentAccessHandler, AgentAccessRequest, AgentAccessState};

/// The bounds every impl in this module shares, named once.
pub trait AgentAccessDispatch:
    GlobalDispatch<CrownosAgentAccessManagerV1, AgentAccessGlobalData>
    + Dispatch<CrownosAgentAccessManagerV1, ()>
    + Dispatch<CrownosAgentAccessV1, ()>
    + AgentAccessHandler
    + 'static
{
}

impl<D> AgentAccessDispatch for D where
    D: GlobalDispatch<CrownosAgentAccessManagerV1, AgentAccessGlobalData>
        + Dispatch<CrownosAgentAccessManagerV1, ()>
        + Dispatch<CrownosAgentAccessV1, ()>
        + AgentAccessHandler
        + 'static
{
}

impl<D: AgentAccessDispatch> GlobalDispatch<CrownosAgentAccessManagerV1, AgentAccessGlobalData, D>
    for AgentAccessState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosAgentAccessManagerV1>,
        _global_data: &AgentAccessGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &AgentAccessGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: AgentAccessDispatch> Dispatch<CrownosAgentAccessManagerV1, (), D> for AgentAccessState {
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &CrownosAgentAccessManagerV1,
        request: crownos_agent_access_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_agent_access_manager_v1::Request::RequestAccess { id, app_id, reason } =
            request
        else {
            return;
        };
        let resource = data_init.init(id, ());
        state.request_agent_access(AgentAccessRequest::new(resource, app_id, reason));
    }
}

impl<D: AgentAccessDispatch> Dispatch<CrownosAgentAccessV1, (), D> for AgentAccessState {
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &CrownosAgentAccessV1,
        _request: crownos_agent_access_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &CrownosAgentAccessV1, _data: &()) {
        if let Some(grant) = state.agent_access_state().release(resource) {
            state.agent_access_released(grant);
        }
    }
}

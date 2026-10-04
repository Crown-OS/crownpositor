//! Delegation glue for `crownos_agent_access_v1`, and the audit trail of every
//! decision. The protocol lives in [`protocols::crownos_agent_access`], the
//! policy in [`crate::state::agent_access`].

use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crownos_protocols::agent_access::v1::server::{
    crownos_agent_access_manager_v1::CrownosAgentAccessManagerV1,
    crownos_agent_access_v1::CrownosAgentAccessV1,
};
use protocols::crownos_agent_access::{
    AgentAccessGlobalData, AgentAccessGrant, AgentAccessHandler, AgentAccessRequest,
    AgentAccessState,
};

use crate::{
    state::{State, agent_access::AgentAccessPolicy},
    utils::token::random_hex_token,
};

const AUDIT: &str = "crownpositor::audit::agent_access";

impl AgentAccessHandler for State {
    fn agent_access_state(&mut self) -> &mut AgentAccessState {
        &mut self.wayland.agent_access_state
    }

    fn request_agent_access(&mut self, request: AgentAccessRequest) {
        if request.app_id().is_empty() {
            return self.deny_agent_access(request, "empty app_id");
        }
        match self.wayland.agent_access_policy {
            AgentAccessPolicy::GrantAll => self.grant_agent_access(request),
            AgentAccessPolicy::DenyAll => self.deny_agent_access(request, "policy"),
        }
    }

    fn agent_access_released(&mut self, grant: AgentAccessGrant) {
        tracing::info!(target: AUDIT, app_id = grant.app_id(), "agent access released");
    }
}

impl State {
    fn grant_agent_access(&mut self, request: AgentAccessRequest) {
        let token = match random_hex_token() {
            Ok(token) => token,
            Err(error) => {
                tracing::error!(target: AUDIT, %error, "no randomness for an agent access token");
                return self.deny_agent_access(request, "token generation failed");
            }
        };
        let pid = self.requester_pid(&request);
        let app_id = request.app_id().to_owned();
        let reason = request.reason().to_owned();
        if request.grant(&mut self.wayland.agent_access_state, token) {
            tracing::info!(target: AUDIT, app_id, ?pid, reason, "agent access granted");
        }
    }

    fn deny_agent_access(&self, request: AgentAccessRequest, cause: &str) {
        tracing::info!(
            target: AUDIT,
            app_id = request.app_id(),
            pid = ?self.requester_pid(&request),
            reason = request.reason(),
            cause,
            "agent access denied"
        );
        request.deny();
    }

    fn requester_pid(&self, request: &AgentAccessRequest) -> Option<i32> {
        let credentials = request
            .client()?
            .get_credentials(&self.common.display_handle)
            .ok()?;
        Some(credentials.pid)
    }
}

delegate_global_dispatch!(State: [CrownosAgentAccessManagerV1: AgentAccessGlobalData] => AgentAccessState);
delegate_dispatch!(State: [CrownosAgentAccessManagerV1: ()] => AgentAccessState);
delegate_dispatch!(State: [CrownosAgentAccessV1: ()] => AgentAccessState);

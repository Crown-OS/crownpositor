//! One request awaiting an answer, and the grant it may become.

use crownos_protocols::agent_access::v1::server::crownos_agent_access_v1::CrownosAgentAccessV1;
use wayland_server::{Client, Resource};

use super::AgentAccessState;

/// A request the compositor has not answered yet. Dropping it unanswered
/// leaves the client waiting, which is what a prompt still on screen does.
#[derive(Debug)]
pub struct AgentAccessRequest {
    resource: CrownosAgentAccessV1,
    app_id: String,
    reason: String,
}

impl AgentAccessRequest {
    pub(super) fn new(resource: CrownosAgentAccessV1, app_id: String, reason: String) -> Self {
        Self {
            resource,
            app_id,
            reason,
        }
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn client(&self) -> Option<Client> {
        self.resource.client()
    }

    /// Whether the client still holds the object, so an answer would reach it.
    pub fn is_pending(&self) -> bool {
        self.resource.is_alive()
    }

    /// Grants access under `token`. Returns `false`, recording nothing, when
    /// the client withdrew the request before the answer.
    pub fn grant(self, state: &mut AgentAccessState, token: String) -> bool {
        if !self.is_pending() {
            return false;
        }
        self.resource.granted(token.clone());
        state.record(AgentAccessGrant {
            resource: self.resource,
            app_id: self.app_id,
            token,
        });
        true
    }

    pub fn deny(self) {
        if self.is_pending() {
            self.resource.denied();
        }
    }
}

/// Access the compositor granted and has not taken back.
#[derive(Debug)]
pub struct AgentAccessGrant {
    resource: CrownosAgentAccessV1,
    app_id: String,
    token: String,
}

impl AgentAccessGrant {
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub(super) fn token(&self) -> &str {
        &self.token
    }

    pub(super) fn is_for(&self, resource: &CrownosAgentAccessV1) -> bool {
        self.resource == *resource
    }

    pub(super) fn send_revoked(&self) {
        if self.resource.is_alive() {
            self.resource.revoked();
        }
    }
}

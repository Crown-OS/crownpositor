//! Server-side `crownos_agent_access_v1`.
//!
//! The protocol half records requests and grants and nothing else. Whether a
//! request is granted is the compositor's decision, made in
//! [`AgentAccessHandler::request_agent_access`], now or once the user has
//! answered. Live grants are kept here, so their tokens can be checked and
//! revoked; no per-object state lives in user data, so none of it needs a lock.

mod dispatch;
mod request;

use crownos_protocols::agent_access::v1::server::{
    crownos_agent_access_manager_v1::CrownosAgentAccessManagerV1,
    crownos_agent_access_v1::CrownosAgentAccessV1,
};
use wayland_server::{Client, DisplayHandle, backend::GlobalId};

pub use dispatch::AgentAccessDispatch;
pub use request::{AgentAccessGrant, AgentAccessRequest};

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`AgentAccessState`].
pub trait AgentAccessHandler {
    fn agent_access_state(&mut self) -> &mut AgentAccessState;

    /// Answers with [`AgentAccessRequest::grant`] or
    /// [`AgentAccessRequest::deny`], immediately or later. The protocol
    /// requires denying a request whose app_id is empty.
    fn request_agent_access(&mut self, request: AgentAccessRequest);

    /// A grant ended because its client destroyed the object or disconnected.
    fn agent_access_released(&mut self, grant: AgentAccessGrant);
}

/// Global data: which clients may see the manager.
pub struct AgentAccessGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Delegate type for the `crownos_agent_access_manager_v1` global.
pub struct AgentAccessState {
    global: GlobalId,
    grants: Vec<AgentAccessGrant>,
}

impl AgentAccessState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: AgentAccessDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosAgentAccessManagerV1, _>(
            VERSION,
            AgentAccessGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            grants: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Whether `token` belongs to a grant that is still live.
    pub fn is_live_token(&self, token: &str) -> bool {
        self.grants.iter().any(|grant| grant.token() == token)
    }

    /// Ends every grant held for `app_id`, returning them.
    pub fn revoke_app(&mut self, app_id: &str) -> Vec<AgentAccessGrant> {
        let (revoked, kept) = std::mem::take(&mut self.grants)
            .into_iter()
            .partition(|grant| grant.app_id() == app_id);
        self.grants = kept;
        for grant in &revoked {
            grant.send_revoked();
        }
        revoked
    }

    fn record(&mut self, grant: AgentAccessGrant) {
        self.grants.push(grant);
    }

    fn release(&mut self, resource: &CrownosAgentAccessV1) -> Option<AgentAccessGrant> {
        let index = self
            .grants
            .iter()
            .position(|grant| grant.is_for(resource))?;
        Some(self.grants.swap_remove(index))
    }
}

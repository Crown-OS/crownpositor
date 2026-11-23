//! Delegation glue for `crownos_agent_access_v1`, and the audit trail of every
//! decision. The protocol lives in [`protocols::crownos_agent_access`], the
//! policy in [`crate::state::agent_access`], the dialog in
//! [`crate::utils::consent_questions`].

use crownos_config::schema::{AccessDecision, Agents};
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
    utils::{
        consent_questions::{AccessAnswer, access_answer, agent_access_question},
        token::random_hex_token,
    },
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
            AgentAccessPolicy::Ask => self.decide_agent_access(request),
        }
    }

    fn agent_access_released(&mut self, grant: AgentAccessGrant) {
        tracing::info!(target: AUDIT, app_id = grant.app_id(), "agent access released");
    }
}

impl State {
    fn decide_agent_access(&mut self, request: AgentAccessRequest) {
        let agents = &self.config.current.agents;
        match agents.app_access.get(request.app_id()) {
            Some(AccessDecision::Allow) => self.grant_agent_access(request),
            Some(AccessDecision::Deny) => self.deny_agent_access(request, "remembered"),
            None if !agents.ask_for_app_access => self.deny_agent_access(request, "policy"),
            None if self
                .agent_prompts
                .is_asking(|asked| asked.app_id() == request.app_id()) =>
            {
                self.deny_agent_access(request, "already asking")
            }
            None => self.prompt_agent_access(request),
        }
    }

    fn prompt_agent_access(&mut self, request: AgentAccessRequest) {
        let question = agent_access_question(request.app_id(), request.reason());
        let app_id = request.app_id().to_owned();
        let ticket = self.agent_prompts.park(request);
        tracing::info!(target: AUDIT, app_id, ticket, "asking the user");
        self.ask_consent(question, move |state, answer| {
            state.answer_agent_access(ticket, access_answer(&answer));
        });
    }

    fn answer_agent_access(&mut self, ticket: u64, answer: Result<AccessAnswer, String>) {
        let Some(request) = self.agent_prompts.take(ticket) else {
            return;
        };
        match answer {
            Ok(AccessAnswer { granted, remember }) => {
                if remember {
                    let decision = if granted {
                        AccessDecision::Allow
                    } else {
                        AccessDecision::Deny
                    };
                    self.remember_agent_access(request.app_id(), decision);
                }
                if granted {
                    self.grant_agent_access(request);
                } else {
                    self.deny_agent_access(request, "user");
                }
            }
            Err(error) => {
                tracing::warn!(target: AUDIT, %error, "nobody could be asked");
                self.deny_agent_access(request, "no prompt");
            }
        }
    }

    /// Writes the answer into `agents.ron`; the config watch brings it back.
    fn remember_agent_access(&self, app_id: &str, decision: AccessDecision) {
        let mut agents: Agents = crownos_config::load(Agents::SECTION);
        agents.app_access.insert(app_id.to_owned(), decision);
        if let Err(error) = crownos_config::save(Agents::SECTION, &agents) {
            tracing::warn!(target: AUDIT, %error, "cannot remember the agent access answer");
        }
    }

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

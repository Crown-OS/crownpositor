//! Delegation glue for `crownos_agent_cursor_v1`: the protocol lives in
//! [`protocols::crownos_agent_cursor`], the seats in
//! [`crate::state::agent_seats`].

use crownos_config::schema::Scope as GrantScope;
use crownos_protocols::agent_cursor::v1::server::{
    crownos_agent_cursor_manager_v1::CrownosAgentCursorManagerV1,
    crownos_agent_cursor_v1::CrownosAgentCursorV1,
};
use protocols::crownos_agent_cursor::{
    AgentCursor, AgentCursorGlobalData, AgentCursorHandler, AgentCursorState, CreateCursorError,
    CursorData, CursorFrame,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::state::State;
use crate::utils::privilege::may_drive_agents;

const AUDIT: &str = "crownpositor::audit::agent_cursor";

impl AgentCursorHandler for State {
    fn agent_cursor_state(&mut self) -> &mut AgentCursorState {
        &mut self.wayland.agent_cursor_state
    }

    /// The global's filter checks the grant at bind only: a client whose grant went away keeps
    /// its manager, so each new cursor is checked again.
    fn create_agent_cursor(
        &mut self,
        cursor: AgentCursor,
        name: &str,
        color: u32,
    ) -> Result<String, CreateCursorError> {
        let allowed = cursor.client().is_some_and(|client| {
            may_drive_agents(
                &client,
                &self.wayland.context_grants,
                &self.wayland.trusted_executables,
                GrantScope::WaylandAgentCursor,
            )
        });
        if !allowed {
            tracing::warn!(target: AUDIT, "refused an agent cursor: the client has no agent cursor grant");
            return Err(CreateCursorError::Refused);
        }
        self.create_agent_seat(cursor, name, color)
    }

    fn agent_cursor_frame(&mut self, cursor: &AgentCursor, frame: CursorFrame) {
        self.apply_agent_frame(cursor, frame);
    }

    fn agent_cursor_focus(&mut self, cursor: &AgentCursor, identifier: &str) {
        self.focus_agent_window(cursor, identifier);
    }

    fn agent_cursor_destroyed(&mut self, cursor: &AgentCursor) {
        self.destroy_agent_seat(cursor);
    }
}

delegate_global_dispatch!(State: [CrownosAgentCursorManagerV1: AgentCursorGlobalData] => AgentCursorState);
delegate_dispatch!(State: [CrownosAgentCursorManagerV1: ()] => AgentCursorState);
delegate_dispatch!(State: [CrownosAgentCursorV1: CursorData] => AgentCursorState);

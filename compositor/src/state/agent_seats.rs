//! The seats behind `crownos_agent_cursor_v1`: one pointer and keyboard per
//! agent, confined to the agent workspace's output, drawn as a named cursor.
//! How a seat is driven is [`crate::state::driven_seat`]'s.

use protocols::crownos_agent_cursor::{AgentCursor, CreateCursorError, CursorFrame, RevokeReason};
use smithay::utils::SERIAL_COUNTER;

use crate::{
    rendering::{cursor::AgentOverlay, decoration::label::Label},
    state::{
        State,
        driven_seat::{Confinement, DrivenSeat},
    },
};

const AUDIT: &str = "crownpositor::audit::agent_cursor";
/// More agents than this share a desktop badly whatever the compositor does.
const MAX_AGENTS: usize = 8;
const LABEL_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

pub struct AgentSeat {
    cursor: AgentCursor,
    driven: DrivenSeat,
    name: String,
}

#[derive(Default)]
pub struct AgentSeats {
    seats: Vec<AgentSeat>,
    created: u32,
}

impl AgentSeats {
    fn index_of(&self, cursor: &AgentCursor) -> Option<usize> {
        self.seats.iter().position(|seat| seat.cursor == *cursor)
    }

    pub fn is_empty(&self) -> bool {
        self.seats.is_empty()
    }
}

impl State {
    pub fn create_agent_seat(
        &mut self,
        cursor: AgentCursor,
        name: &str,
        color: u32,
    ) -> Result<String, CreateCursorError> {
        if self.agent_seats.seats.len() >= MAX_AGENTS {
            return Err(CreateCursorError::TooMany);
        }
        if self.shell.is_locked() {
            return Err(CreateCursorError::Refused);
        }
        let confinement = self
            .headless_location()
            .and_then(|location| self.shell.monitor_by_id(location.output))
            .map(|monitor| Confinement {
                output: monitor.id(),
                bounds: monitor.geometry(),
            })
            .ok_or(CreateCursorError::Refused)?;
        self.agent_seats.created += 1;
        let seat_name = format!("agent-{}", self.agent_seats.created);
        let driven = self
            .new_driven_seat(&seat_name, confinement)
            .ok_or(CreateCursorError::Refused)?;
        let labels = self.agent_labels(name);
        self.input
            .cursor
            .agents
            .push(AgentOverlay::new(driven.position, color, labels));
        self.agent_seats.seats.push(AgentSeat {
            cursor,
            driven,
            name: name.to_owned(),
        });
        tracing::info!(target: AUDIT, agent = name, seat = %seat_name, "agent cursor created");
        self.queue_redraw();
        Ok(seat_name)
    }

    /// The agent's name rasterised at every output's scale.
    fn agent_labels(&mut self, name: &str) -> Vec<(f64, Label)> {
        let scales: Vec<f64> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| monitor.output().current_scale().fractional_scale())
            .collect();
        scales
            .into_iter()
            .filter_map(|scale| {
                self.text
                    .label(name, scale, LABEL_COLOR, true)
                    .cloned()
                    .map(|label| (scale, label))
            })
            .collect()
    }

    fn agent_confinement(&self) -> Option<Confinement> {
        let output = self.agent_output_id()?;
        let monitor = self.shell.monitor_by_id(output)?;
        Some(Confinement {
            output,
            bounds: monitor.geometry(),
        })
    }

    pub fn apply_agent_frame(&mut self, cursor: &AgentCursor, frame: CursorFrame) {
        let Some(index) = self.agent_seats.index_of(cursor) else {
            return;
        };
        let Some(confinement) = self.agent_confinement() else {
            return;
        };
        let mut agent = self.agent_seats.seats.remove(index);
        let focus = agent.driven.focus;
        self.drive_seat(&mut agent.driven, frame, confinement);
        if let Some(overlay) = self.input.cursor.agents.get_mut(index) {
            overlay.position = agent.driven.position;
        }
        if agent.driven.focus != focus {
            self.announce_agent_focus(&agent);
        }
        self.agent_seats.seats.insert(index, agent);
    }

    fn announce_agent_focus(&self, agent: &AgentSeat) {
        let identifier = agent
            .driven
            .focus
            .and_then(|id| self.wayland.foreign_toplevels.identifier(id))
            .unwrap_or_default();
        agent.cursor.focus_changed(&identifier);
    }

    pub fn focus_agent_window(&mut self, cursor: &AgentCursor, identifier: &str) {
        let Some(index) = self.agent_seats.index_of(cursor) else {
            return;
        };
        let Some(id) = self.wayland.foreign_toplevels.window(identifier) else {
            return;
        };
        let Some(confinement) = self.agent_confinement() else {
            return;
        };
        let mut agent = self.agent_seats.seats.remove(index);
        if let Some(keyboard) = agent.driven.seat.get_keyboard() {
            let focus = agent.driven.focus;
            let serial = SERIAL_COUNTER.next_serial();
            self.focus_driven_seat(&mut agent.driven, &keyboard, id, serial, confinement);
            if agent.driven.focus != focus {
                self.announce_agent_focus(&agent);
            }
        }
        self.agent_seats.seats.insert(index, agent);
    }

    /// Releases whatever the agent held, drops its focus and takes its seat
    /// away.
    pub fn destroy_agent_seat(&mut self, cursor: &AgentCursor) {
        let Some(index) = self.agent_seats.index_of(cursor) else {
            return;
        };
        let mut removed = self.agent_seats.seats.remove(index);
        self.release_driven_seat(&mut removed.driven);
        if index < self.input.cursor.agents.len() {
            self.input.cursor.agents.remove(index);
        }
        tracing::info!(target: AUDIT, agent = removed.name, "agent cursor destroyed");
    }

    /// Takes every agent's seat away, as the user's shortcut and a session
    /// lock do.
    pub fn revoke_agents(&mut self, reason: RevokeReason) {
        let cursors = self.wayland.agent_cursor_state.revoke_all(reason);
        for cursor in &cursors {
            self.destroy_agent_seat(cursor);
        }
    }

    /// A locked session has no agents at work in it.
    pub fn revoke_agents_if_locked(&mut self) {
        if self.shell.is_locked() && !self.agent_seats.is_empty() {
            self.revoke_agents(RevokeReason::Locked);
        }
    }
}

//! Server-side `crownos_agent_cursor_v1`: a seat of an agent's own.
//!
//! Like the injector, a cursor buffers its input requests until `frame` and
//! hands them to the compositor as one [`CursorFrame`]. What a seat is, where
//! its pointer may go and how it is drawn is the compositor's business through
//! [`AgentCursorHandler`]; this module only validates and routes.

mod dispatch;

use std::sync::Mutex;

use crownos_protocols::agent_cursor::v1::server::{
    crownos_agent_cursor_manager_v1::CrownosAgentCursorManagerV1,
    crownos_agent_cursor_v1::CrownosAgentCursorV1,
};
use smithay::utils::{Logical, Point};
use wayland_server::{Client, DisplayHandle, Resource, backend::GlobalId};

pub use crownos_protocols::agent_cursor::v1::server::crownos_agent_cursor_v1::{
    Axis, ButtonState, KeyState, RevokeReason,
};
pub use dispatch::AgentCursorDispatch;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;
pub const MAX_NAME_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorEvent {
    Motion { delta: Point<f64, Logical> },
    MotionAbsolute { position: Point<f64, Logical> },
    Button { button: u32, state: ButtonState },
    Axis { axis: Axis, value: f64 },
    Key { key: u32, state: KeyState },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CursorFrame {
    pub events: Vec<CursorEvent>,
    /// CLOCK_MONOTONIC milliseconds, 0 for now.
    pub time_ms: u32,
}

/// Why a cursor could not be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateCursorError {
    TooMany,
    Refused,
}

/// What the compositor provides for this protocol to be delegated to
/// [`AgentCursorState`].
pub trait AgentCursorHandler {
    fn agent_cursor_state(&mut self) -> &mut AgentCursorState;
    /// Creates the seat behind `cursor`, answering with its `wl_seat` name.
    fn create_agent_cursor(
        &mut self,
        cursor: AgentCursor,
        name: &str,
        color: u32,
    ) -> Result<String, CreateCursorError>;
    fn agent_cursor_frame(&mut self, cursor: &AgentCursor, frame: CursorFrame);
    fn agent_cursor_focus(&mut self, cursor: &AgentCursor, identifier: &str);
    /// The client destroyed the cursor or went away; the seat goes with it.
    fn agent_cursor_destroyed(&mut self, cursor: &AgentCursor);
}

/// Global data: which clients may see the manager.
pub struct AgentCursorGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// User data of a `crownos_agent_cursor_v1`: the frame being built.
#[derive(Debug, Default)]
pub struct CursorData {
    pending: Mutex<Vec<CursorEvent>>,
}

impl CursorData {
    fn push(&self, event: CursorEvent) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }

    fn take(&self) -> Vec<CursorEvent> {
        std::mem::take(
            &mut self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// The compositor's handle on one cursor, through which its events go out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCursor(CrownosAgentCursorV1);

impl AgentCursor {
    pub fn is_alive(&self) -> bool {
        self.0.is_alive()
    }

    pub fn created(&self, seat: &str) {
        self.0.created(seat.to_owned());
    }

    pub fn focus_changed(&self, identifier: &str) {
        self.0.focus_changed(identifier.to_owned());
    }

    pub fn revoked(&self, reason: RevokeReason) {
        self.0.revoked(reason);
    }
}

/// Delegate type for the `crownos_agent_cursor_manager_v1` global.
pub struct AgentCursorState {
    global: GlobalId,
    cursors: Vec<AgentCursor>,
}

impl AgentCursorState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: AgentCursorDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosAgentCursorManagerV1, _>(
            VERSION,
            AgentCursorGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            cursors: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// The cursors whose seats still exist.
    pub fn cursors(&self) -> &[AgentCursor] {
        &self.cursors
    }

    /// Tells every cursor it is gone and forgets them; the compositor tears
    /// down the seats itself.
    pub fn revoke_all(&mut self, reason: RevokeReason) -> Vec<AgentCursor> {
        let cursors = std::mem::take(&mut self.cursors);
        for cursor in &cursors {
            cursor.revoked(reason);
        }
        cursors
    }

    /// Tells the cursors whose client `revoked` selects that they are gone,
    /// and forgets them; the compositor tears down their seats itself.
    pub fn revoke_where(
        &mut self,
        mut revoked: impl FnMut(&Client) -> bool,
        reason: RevokeReason,
    ) -> Vec<AgentCursor> {
        let (gone, kept) = std::mem::take(&mut self.cursors)
            .into_iter()
            .partition(|cursor| cursor.0.client().is_some_and(|client| revoked(&client)));
        self.cursors = kept;
        for cursor in &gone {
            cursor.revoked(reason);
        }
        gone
    }

    /// The clients that hold a cursor.
    pub fn clients(&self) -> Vec<Client> {
        let mut clients: Vec<Client> = Vec::new();
        for client in self.cursors.iter().filter_map(|cursor| cursor.0.client()) {
            if !clients.iter().any(|known| known.id() == client.id()) {
                clients.push(client);
            }
        }
        clients
    }

    fn record(&mut self, cursor: AgentCursor) {
        self.cursors.push(cursor);
    }

    fn forget(&mut self, cursor: &AgentCursor) -> bool {
        let before = self.cursors.len();
        self.cursors.retain(|known| known != cursor);
        self.cursors.len() != before
    }
}

/// Whether a cursor name is one the protocol allows.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME_LEN && !name.chars().any(char::is_control)
}

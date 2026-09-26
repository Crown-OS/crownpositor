//! Server-side `crownos_screencast_v1`.
//!
//! Captures an output into a ring of client-owned dmabufs. This module is the
//! protocol half: it validates requests, owns the ring's ownership state and
//! sends every event, while the compositor decides when to render and does the
//! rendering. The split mirrors the rest of this crate — nothing here touches
//! a renderer — and it means the rules the XML states (who owns a slot, when a
//! frame may be sent, what a constraints change invalidates) live in one
//! place, next to the tests that pin them.

mod constraints;
mod dispatch;
mod ring;
mod session;

use std::os::fd::OwnedFd;

use crownos_protocols::screencast::v1::server::crownos_screencast_manager_v1::CrownosScreencastManagerV1;
use wayland_server::{Client, DisplayHandle, backend::GlobalId, protocol::wl_output::WlOutput};

pub use constraints::{BufferConstraints, FormatModifiers, join_halves, modifier_halves};
pub use crownos_protocols::screencast::v1::server::{
    crownos_screencast_manager_v1::{Capability, CursorMode},
    crownos_screencast_session_v1::StopReason,
};
pub use dispatch::ScreencastDispatch;
pub use ring::{MAX_SLOTS, Released, RingError, SlotOwner};
pub use session::{ClaimedSlot, CursorImage, ScreencastSession, SessionData, frame_interval};

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`ScreencastState`].
pub trait ScreencastHandler {
    fn screencast_state(&mut self) -> &mut ScreencastState;

    /// A client asked to capture `output`. The compositor sends the first
    /// constraints, or stops the session if the output is unusable.
    fn new_screencast_session(&mut self, session: ScreencastSession, output: &WlOutput);

    /// Something happened that may let a frame be rendered: `start`, a buffer
    /// attached or released, `force_frame`.
    fn screencast_session_ready(&mut self, session: &ScreencastSession);

    /// Imports the session's two DRM syncobj timelines: the compositor
    /// signals `acquire`, the client signals `release`. `false` makes the
    /// request fail with `invalid_timeline`.
    fn import_screencast_timelines(
        &mut self,
        session: &ScreencastSession,
        acquire: OwnedFd,
        release: OwnedFd,
    ) -> bool;

    /// Slot `index` was released with `point` on the session's release
    /// timeline. The compositor calls
    /// [`ScreencastSession::release_signalled`] once it is.
    fn await_screencast_release(&mut self, session: &ScreencastSession, index: usize, point: u64);

    /// The session was stopped by the client or destroyed; the compositor
    /// drops whatever it kept for it.
    fn screencast_session_ended(&mut self, session: &ScreencastSession);
}

/// Global data: which clients may see the manager.
pub struct ScreencastGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Delegate type for the `crownos_screencast_manager_v1` global.
pub struct ScreencastState {
    global: GlobalId,
    capabilities: Capability,
}

impl ScreencastState {
    /// Registers the global, visible only to clients `filter` admits.
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: ScreencastDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosScreencastManagerV1, _>(
            VERSION,
            ScreencastGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            capabilities: Capability::empty(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    pub fn capabilities(&self) -> Capability {
        self.capabilities
    }

    /// What managers bound from now on are told. Set once the backend knows
    /// whether it can import syncobj timelines.
    pub fn set_capabilities(&mut self, capabilities: Capability) {
        self.capabilities = capabilities;
    }
}

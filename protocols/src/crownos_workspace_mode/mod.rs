//! Server-side `crownos_workspace_mode_v1`.
//!
//! The protocol half holds the bound managers and the mode they last heard, so
//! a manager bound later is told at once and [`WorkspaceModeState::update`]
//! only speaks when the mode changed. Until the compositor's first update
//! there is no mode to tell, and a manager bound before it hears the mode
//! from that update. Which workspace is focused, and
//! switching it, are the compositor's business, reached through
//! [`WorkspaceModeHandler`].

mod dispatch;

use crownos_protocols::workspace_mode::v1::server::crownos_workspace_mode_manager_v1::CrownosWorkspaceModeManagerV1;
use wayland_server::{Client, DisplayHandle, backend::GlobalId};

pub use crownos_protocols::workspace_mode::v1::server::crownos_workspace_mode_manager_v1::Mode;
pub use dispatch::WorkspaceModeDispatch;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`WorkspaceModeState`].
pub trait WorkspaceModeHandler {
    fn workspace_mode_state(&mut self) -> &mut WorkspaceModeState;

    /// A client asked for the focused workspace to switch to `mode`. The
    /// answer is the next [`WorkspaceModeState::update`].
    fn set_workspace_mode(&mut self, mode: Mode);
}

/// Global data: which clients may see the manager.
pub struct WorkspaceModeGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Delegate type for the `crownos_workspace_mode_manager_v1` global.
pub struct WorkspaceModeState {
    global: GlobalId,
    managers: Vec<CrownosWorkspaceModeManagerV1>,
    mode: Option<Mode>,
}

impl WorkspaceModeState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: WorkspaceModeDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosWorkspaceModeManagerV1, _>(
            VERSION,
            WorkspaceModeGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            managers: Vec::new(),
            mode: None,
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Tells every bound manager the focused workspace is now in `mode`.
    /// Cheap when nothing changed, so it can follow every scene update.
    pub fn update(&mut self, mode: Mode) {
        if self.mode == Some(mode) {
            return;
        }
        self.mode = Some(mode);
        for manager in &self.managers {
            manager.mode(mode);
        }
    }
}

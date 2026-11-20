//! Server-side `crownos_workspace_mode_v1`.
//!
//! The protocol half holds the bound managers and the mode they last heard, so
//! a manager bound later is told at once and [`WorkspaceModeState::update`]
//! only speaks when the mode changed. Until the compositor's first update
//! there is no mode to tell, and a manager bound before it hears the mode
//! from that update. From version 2 a client can also watch one output, and
//! [`WorkspaceModeState::update_output`] tells those objects about their own
//! output. Which workspace is active, and switching it, are the compositor's
//! business, reached through [`WorkspaceModeHandler`].

mod dispatch;

use crownos_protocols::workspace_mode::v1::server::{
    crownos_workspace_mode_manager_v1::CrownosWorkspaceModeManagerV1,
    crownos_workspace_mode_output_v1::CrownosWorkspaceModeOutputV1,
};
use smithay::output::Output;
use wayland_server::{Client, DisplayHandle, Resource, backend::GlobalId};

pub use crownos_protocols::workspace_mode::v1::server::crownos_workspace_mode_manager_v1::Mode;
pub use dispatch::WorkspaceModeDispatch;

/// The protocol version this module speaks.
pub const VERSION: u32 = 2;

/// What the compositor provides for this protocol to be delegated to
/// [`WorkspaceModeState`].
pub trait WorkspaceModeHandler {
    fn workspace_mode_state(&mut self) -> &mut WorkspaceModeState;

    /// A client asked for the focused workspace to switch to `mode`. The
    /// answer is the next [`WorkspaceModeState::update`].
    fn set_workspace_mode(&mut self, mode: Mode);

    /// The mode of `output`'s active workspace, told to an object the moment
    /// it is created. `None` for an output the compositor does not show.
    fn output_workspace_mode(&mut self, output: &Output) -> Option<Mode>;

    /// A client asked for `output`'s active workspace to switch to `mode`.
    /// The answer is the next [`WorkspaceModeState::update_output`].
    fn set_output_workspace_mode(&mut self, output: &Output, mode: Mode);
}

/// Global data: which clients may see the manager.
pub struct WorkspaceModeGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// One `crownos_workspace_mode_output_v1` and what it was last told.
struct OutputWatch {
    object: CrownosWorkspaceModeOutputV1,
    /// `None` once the output is gone: the object is then inert.
    output: Option<Output>,
    mode: Option<Mode>,
}

/// Delegate type for the `crownos_workspace_mode_manager_v1` global.
pub struct WorkspaceModeState {
    global: GlobalId,
    managers: Vec<CrownosWorkspaceModeManagerV1>,
    mode: Option<Mode>,
    watches: Vec<OutputWatch>,
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
            watches: Vec::new(),
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

    /// Tells every object watching `output` that its active workspace is now
    /// in `mode`. Cheap when nothing changed.
    pub fn update_output(&mut self, output: &Output, mode: Mode) {
        for watch in &mut self.watches {
            if watch.output.as_ref() == Some(output) && watch.mode != Some(mode) {
                watch.mode = Some(mode);
                watch.object.mode(mode);
            }
        }
    }

    /// Makes the objects watching an output that went away inert.
    pub fn output_removed(&mut self, output: &Output) {
        for watch in &mut self.watches {
            if watch.output.as_ref() == Some(output) {
                watch.output = None;
            }
        }
    }

    fn watched_output(&self, object: &CrownosWorkspaceModeOutputV1) -> Option<Output> {
        self.watches
            .iter()
            .find(|watch| watch.object.id() == object.id())
            .and_then(|watch| watch.output.clone())
    }
}

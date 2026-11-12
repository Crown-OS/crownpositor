//! Delegation glue for `crownos_workspace_mode_v1`. The protocol lives in
//! [`protocols::crownos_workspace_mode`]; switching goes through the same
//! action as the keyboard shortcut, so both behave alike.

use crownos_protocols::workspace_mode::v1::server::crownos_workspace_mode_manager_v1::CrownosWorkspaceModeManagerV1;
use protocols::crownos_workspace_mode::{
    Mode, WorkspaceModeGlobalData, WorkspaceModeHandler, WorkspaceModeState,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{input::shortcuts::Action, layout::WorkspaceMode, state::State};

impl WorkspaceModeHandler for State {
    fn workspace_mode_state(&mut self) -> &mut WorkspaceModeState {
        &mut self.wayland.workspace_mode_state
    }

    fn set_workspace_mode(&mut self, mode: Mode) {
        let mode = match mode {
            Mode::Tiling => WorkspaceMode::Tiling,
            Mode::Floating => WorkspaceMode::Floating,
            _ => return,
        };
        self.handle_action(Action::SetWorkspaceMode(mode));
    }
}

impl State {
    /// Tells `crownos_workspace_mode_v1` clients the focused workspace's mode.
    /// Cheap when nothing changed: the protocol state compares before sending.
    pub fn refresh_workspace_mode(&mut self) {
        let Some(mode) = self
            .shell
            .focused_monitor()
            .map(|monitor| monitor.active().mode())
        else {
            return;
        };
        self.wayland.workspace_mode_state.update(match mode {
            WorkspaceMode::Tiling => Mode::Tiling,
            WorkspaceMode::Floating => Mode::Floating,
        });
    }
}

delegate_global_dispatch!(State: [CrownosWorkspaceModeManagerV1: WorkspaceModeGlobalData] => WorkspaceModeState);
delegate_dispatch!(State: [CrownosWorkspaceModeManagerV1: ()] => WorkspaceModeState);

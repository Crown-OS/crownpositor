//! Delegation glue for `crownos_workspace_mode_v1`. The protocol lives in
//! [`protocols::crownos_workspace_mode`]; the manager's switch goes through
//! the same action as the keyboard shortcut, so both behave alike.

use crownos_protocols::workspace_mode::v1::server::{
    crownos_workspace_mode_manager_v1::CrownosWorkspaceModeManagerV1,
    crownos_workspace_mode_output_v1::CrownosWorkspaceModeOutputV1,
};
use protocols::crownos_workspace_mode::{
    Mode, WorkspaceModeGlobalData, WorkspaceModeHandler, WorkspaceModeState,
};
use smithay::output::Output;
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{
    input::shortcuts::Action, layout::WorkspaceMode, shell::monitor::Monitor, state::State,
};

fn protocol_mode(mode: WorkspaceMode) -> Mode {
    match mode {
        WorkspaceMode::Tiling => Mode::Tiling,
        WorkspaceMode::Floating => Mode::Floating,
    }
}

fn workspace_mode(mode: Mode) -> Option<WorkspaceMode> {
    match mode {
        Mode::Tiling => Some(WorkspaceMode::Tiling),
        Mode::Floating => Some(WorkspaceMode::Floating),
        _ => None,
    }
}

impl WorkspaceModeHandler for State {
    fn workspace_mode_state(&mut self) -> &mut WorkspaceModeState {
        &mut self.wayland.workspace_mode_state
    }

    fn set_workspace_mode(&mut self, mode: Mode) {
        if let Some(mode) = workspace_mode(mode) {
            self.handle_action(Action::SetWorkspaceMode(mode));
        }
    }

    fn output_workspace_mode(&mut self, output: &Output) -> Option<Mode> {
        self.shell
            .monitor(output)
            .map(|monitor| protocol_mode(monitor.active().mode()))
    }

    fn set_output_workspace_mode(&mut self, output: &Output, mode: Mode) {
        let (Some(mode), Some(id)) = (
            workspace_mode(mode),
            self.shell.monitor(output).map(Monitor::id),
        ) else {
            return;
        };
        if self.shell.set_workspace_mode_on(id, mode) {
            self.shell.refresh();
            self.update_keyboard_focus();
            self.queue_redraw();
        }
    }
}

impl State {
    /// Tells `crownos_workspace_mode_v1` clients the focused workspace's mode,
    /// and each watched output its own. Cheap when nothing changed: the
    /// protocol state compares before sending.
    pub fn refresh_workspace_mode(&mut self) {
        let protocol = &mut self.wayland.workspace_mode_state;
        for monitor in self.shell.monitors() {
            protocol.update_output(monitor.output(), protocol_mode(monitor.active().mode()));
        }
        if let Some(monitor) = self.shell.focused_monitor() {
            protocol.update(protocol_mode(monitor.active().mode()));
        }
    }
}

delegate_global_dispatch!(State: [CrownosWorkspaceModeManagerV1: WorkspaceModeGlobalData] => WorkspaceModeState);
delegate_dispatch!(State: [CrownosWorkspaceModeManagerV1: ()] => WorkspaceModeState);
delegate_dispatch!(State: [CrownosWorkspaceModeOutputV1: ()] => WorkspaceModeState);

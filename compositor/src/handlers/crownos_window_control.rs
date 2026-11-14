//! Delegation glue for `crownos_window_control_v1`: the protocol lives in
//! [`protocols::crownos_window_control`], the window list in
//! [`crate::state::windows`].

use crownos_protocols::window_control::v1::server::{
    crownos_window_control_manager_v1::CrownosWindowControlManagerV1,
    crownos_window_v1::CrownosWindowV1,
};
use protocols::crownos_window_control::{
    WindowControlGlobalData, WindowControlHandler, WindowControlState, WindowKey,
};
use smithay::{
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    utils::{Logical, Point, Rectangle, Size},
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{
    handlers::ext_workspace::{workspace_by_identifier, workspace_identifier},
    shell::Location,
    state::State,
    utils::id::WindowId,
};

const AUDIT: &str = "crownpositor::audit::window_control";

impl WindowControlHandler for State {
    fn window_control_state(&mut self) -> &mut WindowControlState {
        &mut self.wayland.window_control_state
    }

    fn focus_window(&mut self, key: WindowKey) {
        let id = WindowId::from_raw(key);
        tracing::info!(target: AUDIT, %id, "focus");
        if self.shell.focus_window(id) {
            self.shell.switch_to_workspace_of(id);
            self.settle_shell();
        }
    }

    fn close_window(&mut self, key: WindowKey) {
        let id = WindowId::from_raw(key);
        tracing::info!(target: AUDIT, %id, "close");
        if let Some(toplevel) = self.shell.tile(id).and_then(|tile| tile.toplevel()) {
            toplevel.send_close();
        }
    }

    fn move_window_to_workspace(&mut self, key: WindowKey, workspace: &str) {
        let id = WindowId::from_raw(key);
        let Some(workspace) = workspace_by_identifier(&self.shell, workspace) else {
            return;
        };
        let Some(output) = self
            .shell
            .monitors()
            .iter()
            .find(|monitor| monitor.index_of(workspace).is_some())
            .map(|monitor| monitor.id())
        else {
            return;
        };
        tracing::info!(target: AUDIT, %id, %workspace, "move to workspace");
        if self.shell.move_tile(id, Location { output, workspace }) {
            self.settle_shell();
        }
    }

    fn make_window_visible(&mut self, key: WindowKey) {
        let id = WindowId::from_raw(key);
        if !self.is_on_agent_workspace(id) {
            return;
        }
        let Some(to) = self.shell.default_location() else {
            return;
        };
        tracing::info!(target: AUDIT, %id, "make visible");
        if self.shell.move_tile(id, to) {
            self.shell.focus_window(id);
            self.settle_shell();
        }
    }

    fn make_window_hidden(&mut self, key: WindowKey) {
        let id = WindowId::from_raw(key);
        if self.is_on_agent_workspace(id) {
            return;
        }
        let Some(to) = self.headless_location() else {
            return;
        };
        tracing::info!(target: AUDIT, %id, "make hidden");
        if self.shell.move_tile(id, to) {
            self.settle_shell();
        }
    }

    fn set_window_rectangle(&mut self, key: WindowKey, x: i32, y: i32, width: i32, height: i32) {
        let id = WindowId::from_raw(key);
        if width <= 0 || height <= 0 {
            return;
        }
        let location: Point<i32, Logical> = (x, y).into();
        let size: Size<i32, Logical> = (width, height).into();
        let moved = self.shell.move_floating(id, location);
        let resized =
            self.shell
                .resize_floating(id, Rectangle::new(location, size), ResizeEdge::BottomRight);
        if moved || resized {
            self.settle_shell();
        }
    }

    fn agent_workspace_identifier(&mut self) -> String {
        self.headless_location()
            .map(|location| workspace_identifier(location.workspace))
            .unwrap_or_default()
    }
}

impl State {
    /// After a shell change made on a protocol's behalf, what every keyboard
    /// shortcut handler does before returning.
    pub fn settle_shell(&mut self) {
        self.shell.refresh();
        self.update_keyboard_focus();
        self.queue_redraw();
    }
}

delegate_global_dispatch!(State: [CrownosWindowControlManagerV1: WindowControlGlobalData] => WindowControlState);
delegate_dispatch!(State: [CrownosWindowControlManagerV1: ()] => WindowControlState);
delegate_dispatch!(State: [CrownosWindowV1: WindowKey] => WindowControlState);

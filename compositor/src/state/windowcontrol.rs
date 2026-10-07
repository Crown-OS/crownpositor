//! Driving the Alt+Tab switcher from input.
//!
//! The strip lives on the focused monitor — it was raised from the keyboard,
//! which has no position on screen — and owns the pointer and keyboard as a
//! mode while it is up, the same way the overview does.

use smithay::utils::{Logical, Point};

use windowcontrol::Direction;

use crate::{
    layout::WorkspaceMode, shell::workspace::Workspace, state::State, utils::id::WindowId,
};

/// Pixels of touchpad scroll that move the row by one window.
const SCROLL_PER_ENTRY: f64 = 120.0;

impl State {
    pub fn window_control_is_open(&self) -> bool {
        self.shell
            .monitors()
            .iter()
            .any(|monitor| monitor.window_control().is_open())
    }

    /// On a floating workspace, raises the strip on the focused monitor or
    /// moves its selection once it is up. A tiling workspace already shows
    /// every window, so there it just hands focus to the next one.
    pub(super) fn window_control_advance(&mut self, direction: Direction) {
        if self.overview_is_open() {
            return;
        }
        let Some(monitor) = self.shell.focused_monitor_mut() else {
            return;
        };
        if monitor.window_control().is_open() {
            monitor.window_control_mut().advance(direction);
        } else if monitor.active().mode() == WorkspaceMode::Tiling {
            if let Some(next) = next_tile(monitor.active(), direction) {
                self.shell.focus_window(next);
            }
        } else {
            monitor.with_window_control(|control, monitor| control.open(monitor, direction));
        }
    }

    /// Switches to the selected window, which zooms forward as the strip sinks.
    pub(super) fn window_control_commit(&mut self) {
        let committed = self
            .shell
            .monitors_mut()
            .iter_mut()
            .find_map(|monitor| monitor.window_control_mut().commit());
        if let Some(window) = committed {
            self.shell.focus_window(window);
        }
    }

    pub(super) fn window_control_dismiss(&mut self) {
        for monitor in self.shell.monitors_mut() {
            monitor.window_control_mut().dismiss();
        }
    }

    /// Lifts the thumbnail under the pointer. Returns whether the strip took
    /// the motion, in which case no client may see it.
    pub fn window_control_motion(&mut self, at: Point<f64, Logical>) -> bool {
        let Some((monitor, local)) = self.window_control_at(at) else {
            return false;
        };
        if let Some(monitor) = self.shell.monitor_by_id_mut(monitor)
            && monitor.window_control_mut().motion(local)
        {
            self.queue_redraw();
        }
        true
    }

    /// Releases a click on a thumbnail. Anywhere else just belongs to the
    /// strip.
    pub fn window_control_click(&mut self, at: Point<f64, Logical>) -> bool {
        let Some((monitor, local)) = self.window_control_at(at) else {
            return false;
        };
        let committed = self
            .shell
            .monitor_by_id_mut(monitor)
            .and_then(|monitor| monitor.window_control_mut().click(local));
        if let Some(window) = committed {
            self.shell.focus_window(window);
            self.shell.refresh();
            self.update_keyboard_focus();
        }
        self.queue_redraw();
        true
    }

    /// Scrolls the row: a wheel notch is one window, a touchpad drags it
    /// continuously and lets it glide onto the nearest window when lifted.
    pub fn window_control_scroll(&mut self, amount: f64, discrete: bool, stopped: bool) -> bool {
        let Some(monitor) = self
            .shell
            .monitors_mut()
            .iter_mut()
            .find(|monitor| monitor.window_control().is_open())
        else {
            return false;
        };
        let control = monitor.window_control_mut();
        match (discrete, stopped) {
            (true, _) => {
                control.scroll(amount);
                control.release_scroll();
            }
            (false, false) => control.scroll(amount / SCROLL_PER_ENTRY),
            (false, true) => control.release_scroll(),
        }
        self.queue_redraw();
        true
    }

    /// The monitor whose open strip `at` falls on, and `at` in its own space.
    fn window_control_at(
        &self,
        at: Point<f64, Logical>,
    ) -> Option<(crate::utils::id::OutputId, Point<f64, Logical>)> {
        let monitor = self
            .shell
            .monitor_at(at)
            .filter(|monitor| monitor.window_control().is_open())?;
        Some((monitor.id(), at - monitor.geometry().loc.to_f64()))
    }
}

/// The window after the focused one in layout order, wrapping at either end.
fn next_tile(workspace: &Workspace, direction: Direction) -> Option<WindowId> {
    let tiles = workspace.tiles();
    let len = tiles.len();
    let current = workspace
        .focus()
        .and_then(|focused| tiles.iter().position(|tile| tile.id() == focused));
    let index = match (current, direction) {
        (None, _) => 0,
        (Some(at), Direction::Forward) => (at + 1) % len,
        (Some(at), Direction::Backward) => (at + len - 1) % len,
    };
    tiles.get(index).map(|tile| tile.id())
}

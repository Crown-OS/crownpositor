//! Driving the Alt+Tab switcher from input.
//!
//! The stack lives on the focused monitor — it was raised from the keyboard,
//! which has no position on screen — and owns the pointer and keyboard as a
//! mode while it is up, the same way the overview does. In `Shared` scope its
//! cards are the windows of every linked monitor, and picking one on another
//! monitor takes the pointer there.

use smithay::utils::{Logical, Point};

use windowcontrol::Direction;

use crate::{layout::WorkspaceMode, state::State, utils::id::OutputId};

/// Pixels of touchpad scroll that move the row by one window.
const SCROLL_PER_ENTRY: f64 = 120.0;

impl State {
    pub fn window_control_is_open(&self) -> bool {
        self.shell
            .monitors()
            .iter()
            .any(|monitor| monitor.window_control().is_open())
    }

    /// On a floating workspace, raises the stack on the focused monitor or
    /// moves its selection once it is up, wherever the pointer has gone since.
    /// A tiling workspace already shows every window, so there it just hands
    /// focus to the next one.
    pub(super) fn window_control_advance(&mut self, direction: Direction) {
        if self.overview_is_open() {
            return;
        }
        if let Some(open) = self
            .shell
            .monitors_mut()
            .iter_mut()
            .find(|monitor| monitor.window_control().is_open())
        {
            open.window_control_mut().advance(direction);
            return;
        }
        let Some((owner, mode)) = self
            .shell
            .focused_monitor()
            .map(|monitor| (monitor.id(), monitor.active().mode()))
        else {
            return;
        };
        match mode {
            WorkspaceMode::Tiling => {
                if let Some(next) = self.shell.next_window_in_scope(direction) {
                    self.focus_window_on_any_monitor(next);
                }
            }
            WorkspaceMode::Floating => {
                self.shell
                    .with_window_control(owner, |control, monitor, windows| {
                        control.open(monitor, windows, direction)
                    });
            }
        }
    }

    /// Switches to the top card's window, which lands back in its place as
    /// the stack goes.
    pub(super) fn window_control_commit(&mut self) {
        let committed = self
            .shell
            .monitors_mut()
            .iter_mut()
            .find_map(|monitor| monitor.window_control_mut().commit());
        if let Some(window) = committed {
            self.focus_window_on_any_monitor(window);
        }
    }

    pub(super) fn window_control_dismiss(&mut self) {
        for monitor in self.shell.monitors_mut() {
            monitor.window_control_mut().dismiss();
        }
    }

    /// Lifts the card under the pointer. Returns whether the stack took the
    /// motion, in which case no client may see it.
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

    /// Releases a click: on a card it switches to that window, anywhere else
    /// it dismisses the stack.
    pub fn window_control_click(&mut self, at: Point<f64, Logical>) -> bool {
        let Some((monitor, local)) = self.window_control_at(at) else {
            return false;
        };
        let committed = self
            .shell
            .monitor_by_id_mut(monitor)
            .and_then(|monitor| monitor.window_control_mut().click(local));
        if let Some(window) = committed {
            self.focus_window_on_any_monitor(window);
            self.shell.refresh();
            self.update_keyboard_focus();
        }
        self.queue_redraw();
        true
    }

    /// Slides the stack: a wheel notch is one window, a touchpad drags it
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

    /// The monitor whose open stack `at` falls on, and `at` in its own space.
    fn window_control_at(
        &self,
        at: Point<f64, Logical>,
    ) -> Option<(OutputId, Point<f64, Logical>)> {
        let monitor = self
            .shell
            .monitor_at(at)
            .filter(|monitor| monitor.window_control().is_open())?;
        Some((monitor.id(), at - monitor.geometry().loc.to_f64()))
    }
}

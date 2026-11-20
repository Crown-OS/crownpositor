//! Moving focus between monitors on purpose, and keeping the pointer on one.

use smithay::backend::input::InputTime;

use crate::{
    shell::monitor::Monitor,
    state::State,
    utils::{geometry::centre, id::OutputId},
};

impl State {
    /// Explicit focus onto a monitor: the keyboard goes to its active
    /// workspace and the pointer to the middle of the window that has the
    /// keyboard there, or of the monitor when it has none. The warp runs the
    /// motion path, so the monitor becomes the focused one too.
    pub fn focus_output(&mut self, output: OutputId) {
        if !self.shell.focus_keyboard_on(output) {
            return;
        }
        // A window that has just moved here is laid out on this monitor only
        // after a refresh.
        self.shell.refresh();
        let Some(monitor) = self.shell.monitor_by_id(output) else {
            return;
        };
        let geometry = monitor.geometry();
        let target = monitor
            .active()
            .focus()
            .and_then(|id| monitor.active().tile(id))
            .map(|tile| centre(tile.target()) + geometry.loc.to_f64())
            .filter(|window| geometry.to_f64().contains(*window))
            .unwrap_or_else(|| centre(geometry));
        self.warp_pointer(target, InputTime::now());
    }

    /// Starts the session on the primary monitor, keyboard and pointer both.
    pub fn focus_primary(&mut self) {
        if let Some(primary) = self.shell.primary().map(Monitor::id) {
            self.focus_output(primary);
        }
    }

    /// Pulls the pointer back onto a shown monitor when the one it was on
    /// went away, moved or shrank.
    pub fn refresh_pointer_bounds(&mut self) {
        let location = self.input.pointer_location;
        if self.shell.monitor_at(location).is_some() {
            return;
        }
        let clamped = self.clamp_to_outputs(location);
        if clamped != location {
            self.warp_pointer(clamped, InputTime::now());
        }
    }
}

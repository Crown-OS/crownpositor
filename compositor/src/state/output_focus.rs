//! Moving focus between monitors on purpose, and keeping the pointer on one.

use smithay::backend::input::InputTime;

use crate::{state::State, utils::geometry::centre, utils::id::OutputId};

impl State {
    /// Explicit focus onto a monitor: the keyboard goes to its active
    /// workspace and the pointer to the middle of the window that has the
    /// keyboard there, or of the monitor when it has none. The warp runs the
    /// motion path, so the monitor becomes the focused one too.
    pub fn focus_output(&mut self, output: OutputId) {
        if !self.shell.focus_keyboard_on(output) {
            return;
        }
        let Some(monitor) = self.shell.monitor_by_id(output) else {
            return;
        };
        let origin = monitor.geometry().loc;
        let window = monitor
            .active()
            .focus()
            .and_then(|id| monitor.active().tile(id))
            .map(|tile| {
                let frame = tile.target();
                centre(frame) + origin.to_f64()
            });
        let target = window.unwrap_or_else(|| centre(monitor.geometry()));
        self.warp_pointer(target, InputTime::now());
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

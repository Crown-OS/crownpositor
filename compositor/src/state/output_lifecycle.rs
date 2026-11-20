//! The two doors an output passes through at State level. Each backend builds
//! the output with `Shell::add_output` and records it, then calls
//! `output_added`; each removal, whatever its cause, goes through
//! `output_removed`.

use smithay::output::Output;

use crate::state::State;

impl State {
    /// Everything that follows an output the backend has just recorded.
    pub fn output_added(&mut self, output: &Output) {
        self.refresh_output_heads();
        self.queue_redraw();
        tracing::debug!(output = output.name(), "output added");
    }

    /// Takes an output out of the desktop: its captures end, its windows go
    /// to a survivor, and clients see the head list change.
    pub fn output_removed(&mut self, output: &Output) {
        self.capture.output_removed(output);
        self.shell
            .remove_output(&self.common.display_handle, output);
        self.refresh_output_heads();
        self.queue_redraw();
        tracing::debug!(output = output.name(), "output removed");
    }
}

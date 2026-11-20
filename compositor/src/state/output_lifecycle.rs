//! The two doors an output passes through at State level. Each backend builds
//! the output with `Shell::add_output` and records it, then calls
//! `output_added`; each removal, whatever its cause, goes through
//! `output_removed`.

use smithay::output::Output;

use crate::{shell::identity::OfflineHead, state::State};

impl State {
    /// Everything that follows an output the backend has just recorded.
    pub fn output_added(&mut self, output: &Output) {
        self.sync_offline_heads();
        self.apply_output_settings();
        self.refresh_output_heads();
        self.queue_redraw();
        tracing::debug!(output = output.name(), "output added");
    }

    /// Takes an output out of the desktop: its captures end, its windows go
    /// to a survivor, and clients see the head list change.
    pub fn output_removed(&mut self, output: &Output) {
        self.capture.output_removed(output);
        self.wayland.workspace_mode_state.output_removed(output);
        self.shell
            .remove_output(&self.common.display_handle, output);
        self.sync_offline_heads();
        self.apply_output_settings();
        self.refresh_output_heads();
        self.queue_redraw();
        tracing::debug!(output = output.name(), "output removed");
    }

    /// Tells the shell which heads are connected but switched off, so a twin
    /// of one gives up its EDID identity too.
    pub fn sync_offline_heads(&mut self) {
        let heads = self
            .backend
            .disabled_heads()
            .iter()
            .map(|head| OfflineHead::new(&head.name, &head.identity, head.kind))
            .collect();
        self.shell.set_offline_heads(heads);
    }
}

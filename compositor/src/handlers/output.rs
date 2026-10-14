use smithay::{
    output::Output,
    reexports::wayland_server::{Resource, protocol::wl_output::WlOutput},
    wayland::output::OutputHandler,
};

use crate::state::State;

impl OutputHandler for State {
    /// Fires when a client binds a `wl_output`.
    ///
    /// It is the first moment we know a specific client cares about a specific
    /// output, so any of its surfaces that were already on that output need
    /// their `enter` replayed — they entered before the bind existed, so
    /// smithay had nobody to send it to, and having entered once it will not
    /// send it again.
    fn output_bound(&mut self, output: Output, wl_output: WlOutput) {
        let client = wl_output.id();
        self.shell.with_surfaces_on(&output, |surface, _| {
            if surface.id().same_client_as(&client) {
                surface.enter(&wl_output);
            }
        });
        self.wayland
            .ext_workspace_state
            .output_bound(&output, &wl_output);
    }
}

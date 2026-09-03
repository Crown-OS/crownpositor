use smithay::{
    delegate_output,
    output::Output,
    reexports::wayland_server::{Resource, protocol::wl_output::WlOutput},
    wayland::{output::OutputHandler, seat::WaylandFocus},
};

use crate::state::State;

impl OutputHandler for State {
    /// Fires when a client binds a `wl_output`.
    ///
    /// It is the first moment we know a specific client cares about a specific
    /// output, so any of its surfaces that were already on that output need
    /// their `enter` replayed — they were mapped before the bind existed, so
    /// smithay had nobody to send it to.
    fn output_bound(&mut self, output: Output, wl_output: WlOutput) {
        let Some(monitor) = self.shell.monitor(&output) else {
            // Structurally impossible while `Shell::add_output` is the only
            // door; assert rather than silently doing nothing.
            debug_assert!(false, "a wl_output was bound for an unregistered output");
            return;
        };

        let client = wl_output.id();
        let surfaces: Vec<_> = monitor
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.tiles())
            .filter_map(|tile| tile.window().wl_surface())
            .filter(|surface| surface.id().same_client_as(&client))
            .map(|surface| surface.into_owned())
            .collect();

        for surface in surfaces {
            output.enter(&surface);
        }
    }
}

delegate_output!(State);

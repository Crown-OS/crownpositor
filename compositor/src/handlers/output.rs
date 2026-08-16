use smithay::{
    delegate_output, output::Output, reexports::wayland_server::protocol::wl_output::WlOutput,
    wayland::output::OutputHandler,
};

use crate::state::State;

impl OutputHandler for State {
    fn output_bound(&mut self, _output: Output, _wl_output: WlOutput) {}
}

delegate_output!(State);

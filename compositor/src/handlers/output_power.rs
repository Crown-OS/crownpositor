use protocols::{
    delegate_output_power,
    output_power::{OutputPowerHandler, OutputPowerState},
};
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;

use crate::state::State;

impl OutputPowerHandler for State {
    fn output_power_state(&mut self) -> &mut OutputPowerState {
        &mut self.wayland.output_power_state
    }

    fn output_power(&mut self, wl_output: &WlOutput) -> Option<bool> {
        let output = self.output_for(wl_output)?;
        self.backend.output_power(&output)
    }

    fn set_output_power(&mut self, wl_output: &WlOutput, on: bool) -> bool {
        let Some(output) = self.output_for(wl_output) else {
            return false;
        };
        self.backend.set_output_power(&output, on)
    }
}

delegate_output_power!(State);

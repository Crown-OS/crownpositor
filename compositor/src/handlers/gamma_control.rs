use protocols::{
    delegate_gamma_control,
    gamma_control::{GammaControlHandler, GammaControlState, GammaRamps},
};
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;

use crate::state::State;

impl GammaControlHandler for State {
    fn gamma_control_state(&mut self) -> &mut GammaControlState {
        &mut self.wayland.gamma_control_state
    }

    /// An output the compositor is dimming or warming has no gamma to give: a
    /// client would overwrite that ramp, and reset it to identity on release.
    fn gamma_size(&mut self, wl_output: &WlOutput) -> Option<u32> {
        let output = self.output_for(wl_output)?;
        if self.display_gamma.owns(&output) {
            return None;
        }
        self.backend.gamma_size(&output)
    }

    fn set_gamma(&mut self, wl_output: &WlOutput, ramps: Option<GammaRamps>) -> bool {
        let Some(output) = self.output_for(wl_output) else {
            return false;
        };

        let applied = self.backend.set_gamma(&output, ramps.as_ref());
        if applied {
            // Gamma is applied during scanout, so nothing is redrawn — but the
            // screen only changes on the next page flip, and an idle desktop
            // has none coming.
            self.backend.queue_redraw(Some(&output));
        }
        applied
    }
}

delegate_gamma_control!(State);

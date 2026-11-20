//! Delegation glue for `crownos_output_brightness_v1`. The protocol lives in
//! [`protocols::crownos_output_brightness`]; the model is
//! [`Brightness`](crate::brightness::Brightness).

use crownos_protocols::output_brightness::v1::server::{
    crownos_output_brightness_manager_v1::CrownosOutputBrightnessManagerV1,
    crownos_output_brightness_v1::CrownosOutputBrightnessV1,
};
use protocols::crownos_output_brightness::{
    Control as WireControl, OutputBrightness, OutputBrightnessGlobalData, OutputBrightnessHandler,
    OutputBrightnessState,
};
use smithay::output::Output;
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{
    brightness::{Control, Reading},
    state::State,
};

impl OutputBrightnessHandler for State {
    fn output_brightness_state(&mut self) -> &mut OutputBrightnessState {
        &mut self.wayland.output_brightness_state
    }

    fn output_brightness(&mut self, output: &Output) -> Option<OutputBrightness> {
        let output = self.shell.monitor(output)?.id();
        Some(on_the_wire(self.brightness.reading(output)))
    }

    fn set_output_brightness(&mut self, output: &Output, level: u32) -> Option<OutputBrightness> {
        let id = self.shell.monitor(output)?.id();
        let reading = self.brightness.set(id, level)?;
        Some(on_the_wire(reading))
    }

    fn refresh_output_brightness(&mut self, output: &Output) {
        if let Some(monitor) = self.shell.monitor(output) {
            self.brightness.refresh(monitor.id());
        }
    }
}

impl State {
    /// Tells every `crownos_output_brightness_v1` object of `output` about a
    /// change the protocol did not ask for: a key, a probe or the hardware.
    pub fn announce_brightness(&mut self, output: &Output, reading: Reading) {
        self.wayland
            .output_brightness_state
            .update(output, on_the_wire(reading));
    }
}

fn on_the_wire(reading: Reading) -> OutputBrightness {
    let control = match reading.control {
        Control::None => WireControl::None,
        Control::Backlight => WireControl::Backlight,
        Control::Ddc => WireControl::Ddc,
        Control::Software => WireControl::Software,
    };
    OutputBrightness {
        control,
        level: reading.level,
    }
}

delegate_global_dispatch!(State: [CrownosOutputBrightnessManagerV1: OutputBrightnessGlobalData] => OutputBrightnessState);
delegate_dispatch!(State: [CrownosOutputBrightnessManagerV1: ()] => OutputBrightnessState);
delegate_dispatch!(State: [CrownosOutputBrightnessV1: ()] => OutputBrightnessState);

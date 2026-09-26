//! Delegation glue for `crownos_input_v1`: the protocol lives in
//! [`protocols::crownos_input`], injection in [`crate::input::injected`] and
//! capture in [`crate::input::capture`].

use smithay::utils::{Logical, Point};
use wayland_server::{delegate_dispatch, delegate_global_dispatch, protocol::wl_output::WlOutput};

use crownos_protocols::input::v1::server::{
    crownos_input_capture_v1::CrownosInputCaptureV1,
    crownos_input_injector_v1::CrownosInputInjectorV1,
    crownos_input_manager_v1::CrownosInputManagerV1,
};
use protocols::crownos_input::{
    CaptureData, Edge, InjectedFrame, InjectorData, InputCapture, InputGlobalData, InputHandler,
    InputState,
};

use crate::state::State;

impl InputHandler for State {
    fn input_protocol_state(&mut self) -> &mut InputState {
        &mut self.wayland.crownos_input_state
    }

    fn inject_input(&mut self, frame: InjectedFrame) {
        self.apply_injected_frame(frame);
    }

    fn arm_input_capture(&mut self, capture: &InputCapture, edges: Edge) {
        State::arm_input_capture(self, capture, edges);
    }

    fn release_input_capture(
        &mut self,
        capture: &InputCapture,
        output: &WlOutput,
        position: Point<f64, Logical>,
    ) {
        State::release_input_capture(self, capture, Some(output), position);
    }

    fn input_capture_destroyed(&mut self, capture: &InputCapture) {
        State::input_capture_destroyed(self, capture);
    }
}

delegate_global_dispatch!(State: [CrownosInputManagerV1: InputGlobalData] => InputState);
delegate_dispatch!(State: [CrownosInputManagerV1: ()] => InputState);
delegate_dispatch!(State: [CrownosInputInjectorV1: InjectorData] => InputState);
delegate_dispatch!(State: [CrownosInputCaptureV1: CaptureData] => InputState);

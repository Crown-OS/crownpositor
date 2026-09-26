//! Delegation glue for `crownos_virtual_output_v1`: the protocol lives in
//! [`protocols::crownos_virtual_output`], the outputs in
//! [`crate::backend::virtual_output`].

use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crownos_protocols::virtual_output::v1::server::{
    crownos_virtual_output_manager_v1::CrownosVirtualOutputManagerV1,
    crownos_virtual_output_v1::CrownosVirtualOutputV1,
};
use protocols::crownos_virtual_output::{
    VirtualMode, VirtualOutput, VirtualOutputData, VirtualOutputGlobalData, VirtualOutputHandler,
    VirtualOutputState,
};

use crate::state::State;

impl VirtualOutputHandler for State {
    fn virtual_output_state(&mut self) -> &mut VirtualOutputState {
        &mut self.wayland.virtual_output_state
    }

    fn create_virtual_output(&mut self, output: VirtualOutput, name: &str, mode: VirtualMode) {
        State::create_virtual_output(self, output, name, mode);
    }

    fn set_virtual_output_mode(&mut self, output: &VirtualOutput, mode: VirtualMode) {
        State::set_virtual_output_mode(self, output, mode);
    }

    fn request_virtual_output_frame(&mut self, output: &VirtualOutput) {
        State::request_virtual_output_frame(self, output);
    }

    fn destroy_virtual_output(&mut self, output: &VirtualOutput) {
        self.remove_virtual_output(output);
    }
}

delegate_global_dispatch!(State: [CrownosVirtualOutputManagerV1: VirtualOutputGlobalData] => VirtualOutputState);
delegate_dispatch!(State: [CrownosVirtualOutputManagerV1: ()] => VirtualOutputState);
delegate_dispatch!(State: [CrownosVirtualOutputV1: VirtualOutputData] => VirtualOutputState);

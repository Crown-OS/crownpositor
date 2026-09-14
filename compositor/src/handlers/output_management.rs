use protocols::{
    delegate_output_management,
    output_management::{OutputConfigRequest, OutputManagementHandler, OutputManagementState},
};

use crate::state::{State, outputs::ApplyMode};

impl OutputManagementHandler for State {
    fn output_management_state(&mut self) -> &mut OutputManagementState {
        &mut self.wayland.output_management_state
    }

    fn test_output_config(&mut self, request: OutputConfigRequest) -> bool {
        self.reconfigure_outputs(request, ApplyMode::Test)
    }

    fn apply_output_config(&mut self, request: OutputConfigRequest) -> bool {
        self.reconfigure_outputs(request, ApplyMode::Apply)
    }
}


delegate_output_management!(State);

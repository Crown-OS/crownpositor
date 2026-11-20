//! Delegation glue for `crownos_output_info_v1`. The protocol lives in
//! [`protocols::crownos_output_info`]; the snapshot of each monitor is
//! [`Shell::output_info`](crate::shell::Shell::output_info).

use crownos_protocols::output_info::v1::server::{
    crownos_output_info_manager_v1::CrownosOutputInfoManagerV1,
    crownos_output_info_v1::CrownosOutputInfoV1,
};
use protocols::crownos_output_info::{
    OutputInfo, OutputInfoGlobalData, OutputInfoHandler, OutputInfoState,
};
use smithay::output::Output;
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{shell::monitor::Monitor, state::State};

impl OutputInfoHandler for State {
    fn output_info_state(&mut self) -> &mut OutputInfoState {
        &mut self.wayland.output_info_state
    }

    fn output_info(&mut self, output: &Output) -> Option<OutputInfo> {
        let primary = self.shell.primary().map(Monitor::id);
        let monitor = self.shell.monitor(output)?;
        self.shell
            .output_info(monitor, primary, &self.config.current)
    }
}

impl State {
    /// Tells `crownos_output_info_v1` clients what changed about each output
    /// since they were last told. Cheap when nothing changed: the protocol
    /// state compares before sending.
    pub fn refresh_output_info(&mut self) {
        let protocol = &mut self.wayland.output_info_state;
        if !protocol.is_watched() {
            return;
        }
        let primary = self.shell.primary().map(Monitor::id);
        for monitor in self.shell.monitors() {
            if let Some(info) = self
                .shell
                .output_info(monitor, primary, &self.config.current)
            {
                protocol.update(monitor.output(), &info);
            }
        }
    }
}

delegate_global_dispatch!(State: [CrownosOutputInfoManagerV1: OutputInfoGlobalData] => OutputInfoState);
delegate_dispatch!(State: [CrownosOutputInfoManagerV1: ()] => OutputInfoState);
delegate_dispatch!(State: [CrownosOutputInfoV1: ()] => OutputInfoState);

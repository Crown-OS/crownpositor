//! Delegation glue for `crownos_capture_privacy_v1`: the protocol lives in
//! [`protocols::crownos_capture_privacy`]; the exclusion is read when
//! windows are described and when captures are served.

use crownos_protocols::capture_privacy::v1::server::{
    crownos_capture_privacy_manager_v1::CrownosCapturePrivacyManagerV1,
    crownos_surface_privacy_v1::CrownosSurfacePrivacyV1,
};
use protocols::crownos_capture_privacy::{CapturePrivacyHandler, CapturePrivacyState, PrivacyData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::state::State;

impl CapturePrivacyHandler for State {
    fn capture_privacy_state(&mut self) -> &mut CapturePrivacyState {
        &mut self.wayland.capture_privacy_state
    }

    fn surface_privacy_changed(&mut self, _surface: &WlSurface) {
        self.queue_redraw();
    }
}

delegate_global_dispatch!(State: [CrownosCapturePrivacyManagerV1: ()] => CapturePrivacyState);
delegate_dispatch!(State: [CrownosCapturePrivacyManagerV1: ()] => CapturePrivacyState);
delegate_dispatch!(State: [CrownosSurfacePrivacyV1: PrivacyData] => CapturePrivacyState);

//! Delegation glue for `crownos_screencast_v1`: the protocol lives in
//! [`protocols::crownos_screencast`], the rendering in
//! [`crate::backend::capture`].

use std::os::fd::{AsFd, OwnedFd};

use wayland_server::{delegate_dispatch, delegate_global_dispatch, protocol::wl_output::WlOutput};

use crownos_protocols::screencast::v1::server::{
    crownos_screencast_manager_v1::CrownosScreencastManagerV1,
    crownos_screencast_session_v1::CrownosScreencastSessionV1,
};
use protocols::crownos_screencast::{
    ScreencastGlobalData, ScreencastHandler, ScreencastSession, ScreencastState, SessionData,
    StopReason,
};

use crate::{
    backend::capture::{CaptureSession, delivery, sync::ExplicitSync},
    state::State,
};

impl ScreencastHandler for State {
    fn screencast_state(&mut self) -> &mut ScreencastState {
        &mut self.wayland.screencast_state
    }

    fn new_screencast_session(&mut self, session: ScreencastSession, output: &WlOutput) {
        match self.output_for(output) {
            Some(output) => self.capture.insert(CaptureSession::new(session, output)),
            None => session.stop(StopReason::SourceDestroyed),
        }
    }

    fn screencast_session_ready(&mut self, session: &ScreencastSession) {
        if let Some(capture) = self.capture.session_mut(&session.id()) {
            capture.dirty = true;
        }
    }

    fn import_screencast_timelines(
        &mut self,
        session: &ScreencastSession,
        acquire: OwnedFd,
        release: OwnedFd,
    ) -> bool {
        let Some(device) = self.capture.sync_device() else {
            return false;
        };
        let sync = match ExplicitSync::import(device, acquire.as_fd(), release.as_fd()) {
            Ok(sync) => sync,
            Err(err) => {
                tracing::warn!(%err, "failed to import the screencast timelines");
                return false;
            }
        };
        let Some(capture) = self.capture.session_mut(&session.id()) else {
            return false;
        };
        capture.sync = Some(sync);
        true
    }

    fn await_screencast_release(&mut self, session: &ScreencastSession, index: usize, point: u64) {
        delivery::await_release(self, session.id(), index, point);
    }

    fn screencast_session_ended(&mut self, session: &ScreencastSession) {
        self.capture.remove(&session.id());
    }
}

delegate_global_dispatch!(State: [CrownosScreencastManagerV1: ScreencastGlobalData] => ScreencastState);
delegate_dispatch!(State: [CrownosScreencastManagerV1: ()] => ScreencastState);
delegate_dispatch!(State: [CrownosScreencastSessionV1: SessionData] => ScreencastState);

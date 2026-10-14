use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};
use smithay::{
    backend::allocator::Fourcc,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    utils::{Logical, Rectangle},
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use protocols::wlr_screencopy::{
    FrameData, ScreencopyConstraints, ScreencopyFrame, ScreencopyGlobalData, ScreencopyHandler,
    ScreencopyState,
};

use crate::{
    backend::capture::{screencopy, source::CaptureSource},
    state::State,
};

/// Opaque first: an encoder ignores alpha.
const DMABUF_PREFERENCE: [Fourcc; 4] = [
    Fourcc::Xrgb8888,
    Fourcc::Xbgr8888,
    Fourcc::Argb8888,
    Fourcc::Abgr8888,
];

impl ScreencopyHandler for State {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.wayland.screencopy_state
    }

    fn screencopy_constraints(
        &mut self,
        output: &WlOutput,
        region: Option<Rectangle<i32, Logical>>,
    ) -> Option<ScreencopyConstraints> {
        let output = self.output_for(output)?;
        let dmabuf = self
            .capture
            .dmabuf_constraints(&mut self.backend)
            .and_then(|constraints| {
                DMABUF_PREFERENCE.into_iter().find(|code| {
                    constraints
                        .formats
                        .iter()
                        .any(|(format, modifiers)| format == code && !modifiers.is_empty())
                })
            });
        let placement = CaptureSource::Output(output).place(&self.shell)?;
        screencopy::constraints(&placement, region, dmabuf)
    }

    fn screencopy_frame(&mut self, frame: ScreencopyFrame) {
        match self.output_for(&frame.output) {
            Some(output) => self.capture.screencopies.push(frame, output),
            None => frame.fail(),
        }
    }
}

delegate_global_dispatch!(State: [ZwlrScreencopyManagerV1: ScreencopyGlobalData] => ScreencopyState);
delegate_dispatch!(State: [ZwlrScreencopyManagerV1: ()] => ScreencopyState);
delegate_dispatch!(State: [ZwlrScreencopyFrameV1: FrameData] => ScreencopyState);

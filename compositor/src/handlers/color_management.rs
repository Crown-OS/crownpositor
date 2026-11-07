use std::{borrow::Cow, sync::Arc};

use protocols::color_management::reexports::wp_image_description_info_v1::WpImageDescriptionInfoV1;
use protocols::{
    color_management::{
        ColorManagementHandler, ColorManagementState,
        record::{IccData, ImageDescription},
    },
    delegate_color_management,
};
use smithay::reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface};

use crate::{color::pipeline::output_encoding, state::State};

impl ColorManagementHandler for State {
    fn color_management_state(&mut self) -> &mut ColorManagementState {
        &mut self.wayland.color_management_state
    }

    /// What the output scans out: 8-bit SDR in the encoding undescribed
    /// surfaces are already in. The panel's EDID colorimetry is not reported,
    /// because nothing corrects the framebuffer for it — claiming it would
    /// have a careful client convert for a gamut the pixels never reach.
    fn output_image_description(&mut self, wl_output: &WlOutput) -> Option<Arc<ImageDescription>> {
        let output = self.output_for(wl_output)?;
        self.shell.monitor(&output)?;
        Some(
            self.wayland
                .color_management_state
                .intern(output_encoding()),
        )
    }

    /// The output's encoding, which needs no conversion at all. HDR content
    /// is still accepted and tone-mapped down, but a client asking what to
    /// render is told SDR, because that is what the screen shows.
    fn preferred_image_description(
        &mut self,
        surface: &WlSurface,
    ) -> Option<Arc<ImageDescription>> {
        self.shell
            .output_for_surface(surface)
            .or_else(|| self.shell.focused_output())
            .and_then(|output| self.shell.monitor(output))?;
        Some(
            self.wayland
                .color_management_state
                .intern(output_encoding()),
        )
    }

    /// Refused for now: the renderer has no ICC pipeline, and telling a client
    /// its profile was accepted when nothing honours it would be worse than
    /// saying no.
    fn accept_icc(&mut self, _icc: &IccData) -> Result<(), Cow<'static, str>> {
        Err(Cow::Borrowed(
            "ICC profiles are not yet applied by this compositor",
        ))
    }

    fn defer_image_description_info(
        &mut self,
        object: WpImageDescriptionInfoV1,
        description: Arc<ImageDescription>,
    ) {
        self.common.event_loop_handle.insert_idle(move |_| {
            protocols::color_management::send_image_description_info(&object, &description);
        });
    }

    fn surface_color_changed(&mut self, surface: &WlSurface) {
        self.shell.described_surfaces.track(surface);
        if let Some(output) = self.shell.output_for_surface(surface).cloned() {
            self.backend.queue_redraw(Some(&output));
        }
    }
}

delegate_color_management!(State);

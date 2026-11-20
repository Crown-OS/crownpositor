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

use crate::state::State;

impl ColorManagementHandler for State {
    fn color_management_state(&mut self) -> &mut ColorManagementState {
        &mut self.wayland.color_management_state
    }

    /// What the output scans out, from its own colour state. The panel's EDID
    /// colorimetry is not reported, because nothing corrects the framebuffer
    /// for it: claiming it would have a careful client convert for a gamut
    /// the pixels never reach.
    fn output_image_description(&mut self, wl_output: &WlOutput) -> Option<Arc<ImageDescription>> {
        let output = self.output_for(wl_output)?;
        let description = self.shell.monitor(&output)?.color().description();
        Some(self.wayland.color_management_state.intern(description))
    }

    /// The description of the output the surface is on, which needs no
    /// conversion at all. HDR content is still accepted and tone-mapped down,
    /// but a client asking what to render is told what its screen shows.
    fn preferred_image_description(
        &mut self,
        surface: &WlSurface,
    ) -> Option<Arc<ImageDescription>> {
        let description = self
            .shell
            .output_for_surface(surface)
            .or_else(|| self.shell.focused_output())
            .and_then(|output| self.shell.monitor(output))?
            .color()
            .description();
        Some(self.wayland.color_management_state.intern(description))
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

impl State {
    /// Re-evaluates every surface's preferred description once something
    /// marked them stale: a window or a monitor came or went, or an output's
    /// colours changed.
    pub fn refresh_preferred_descriptions(&mut self) {
        if self.shell.take_preferred_stale() {
            ColorManagementState::refresh_preferred(self);
        }
    }
}

delegate_color_management!(State);

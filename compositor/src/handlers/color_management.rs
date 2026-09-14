use std::{borrow::Cow, sync::Arc};

use protocols::{
    color_management::{
        ColorManagementHandler, ColorManagementState,
        math::{Chromaticity, Primaries, named},
        record::{
            DescriptionKind, IccData, ImageDescription, NamedPrimaries, NamedTransferFunction,
            ParametricDescription, PrimariesSpec, TransferFunction,
        },
    },
    delegate_color_management,
};
use smithay::reexports::wayland_server::protocol::{
    wl_output::WlOutput, wl_surface::WlSurface,
};
use protocols::color_management::reexports::wp_image_description_info_v1::WpImageDescriptionInfoV1;

use crate::state::State;

impl ColorManagementHandler for State {
    fn color_management_state(&mut self) -> &mut ColorManagementState {
        &mut self.wayland.color_management_state
    }

    fn output_image_description(&mut self, wl_output: &WlOutput) -> Option<Arc<ImageDescription>> {
        let output = self.output_for(wl_output)?;
        let kind = self.shell.monitor(&output).map(output_description)?;
        Some(self.wayland.color_management_state.intern(kind))
    }

    /// What a surface should ideally render into.
    ///
    /// The output's own description today. Once an output can be driven in
    /// BT.2100 this becomes PQ for an HDR output, because that is what scans
    /// out with no conversion at all.
    fn preferred_image_description(
        &mut self,
        surface: &WlSurface,
    ) -> Option<Arc<ImageDescription>> {
        let monitor = self
            .shell
            .output_for_surface(surface)
            .or_else(|| self.shell.focused_output())
            .and_then(|output| self.shell.monitor(output))?;

        let kind = output_description(monitor);
        Some(self.wayland.color_management_state.intern(kind))
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
        if let Some(output) = self.shell.output_for_surface(surface).cloned() {
            self.backend.queue_redraw(Some(&output));
        }
    }
}

/// The colours a monitor actually shows.
///
/// Derived from its EDID where that is trustworthy: the reported chromaticity
/// has already been sanity-checked by the parser, so anything that survives is
/// better than assuming sRGB. A panel that said nothing usable gets sRGB,
/// which is what an undescribed surface is defined to be anyway.
fn output_description(monitor: &crate::shell::monitor::Monitor) -> DescriptionKind {
    let edid = monitor.config().edid.as_ref();

    let primaries = edid
        .and_then(|edid| edid.chromaticity)
        .map(|chromaticity| {
            let point = |(x, y): (f32, f32)| Chromaticity::new(f64::from(x), f64::from(y));
            PrimariesSpec::Raw(Primaries {
                red: point(chromaticity.red),
                green: point(chromaticity.green),
                blue: point(chromaticity.blue),
                white: point(chromaticity.white),
            })
        })
        .unwrap_or(PrimariesSpec::Named(NamedPrimaries::Srgb));

    // The EDID's gamma where it is close to a curve we can name, a power
    // curve otherwise. `gamma22` rather than the deprecated `srgb` entry.
    let transfer = match edid.and_then(|edid| edid.gamma) {
        Some(gamma) if (gamma - 2.2).abs() < 0.05 => {
            TransferFunction::Named(NamedTransferFunction::Gamma22)
        }
        Some(gamma) if (gamma - 2.8).abs() < 0.05 => {
            TransferFunction::Named(NamedTransferFunction::Gamma28)
        }
        Some(gamma) => TransferFunction::Power((gamma as f64 * 10_000.0).round() as u32),
        None => TransferFunction::Named(NamedTransferFunction::Gamma22),
    };

    let luminances = NamedTransferFunction::Gamma22.default_luminances();

    DescriptionKind::Parametric(ParametricDescription {
        primaries,
        transfer,
        luminances,
        target_primaries: match primaries {
            PrimariesSpec::Raw(raw) => raw,
            PrimariesSpec::Named(_) => named::SRGB,
        },
        target_luminance: (luminances.min_scaled, luminances.max),
        max_cll: None,
        max_fall: None,
    })
}

delegate_color_management!(State);

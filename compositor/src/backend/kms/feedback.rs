//! Per-surface `zwp_linux_dmabuf_feedback_v1`: which formats let a client's
//! buffer skip composition on the output it is shown on.

use smithay::{
    backend::{
        allocator::format::FormatSet,
        drm::{DrmNode, DrmSurface},
    },
    reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags,
    wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder},
};

/// The two answers a surface can be given: allocate for the compositing GPU,
/// or for the planes of the output it is on.
#[derive(Debug)]
pub struct SurfaceFeedback {
    pub render: DmabufFeedback,
    pub scanout: DmabufFeedback,
}

impl SurfaceFeedback {
    /// `sampled` is what the compositing GPU can read, `rendered` what the
    /// output's own GPU can; the scanout tranche is limited to what a plane
    /// takes *and* the compositor could still draw, so a buffer that misses
    /// the plane never goes blank.
    pub fn new(
        compositing: DrmNode,
        sampled: FormatSet,
        scanout: DrmNode,
        rendered: FormatSet,
        surface: &DrmSurface,
    ) -> std::io::Result<Self> {
        let drawable: FormatSet = sampled.iter().chain(rendered.iter()).copied().collect();
        let plane_formats = scanout_formats(surface, &drawable);

        let builder = DmabufFeedbackBuilder::new(compositing.dev_id(), sampled);
        Ok(Self {
            render: builder
                .clone()
                .add_preference_tranche(
                    scanout.dev_id(),
                    TrancheFlags::Sampling,
                    rendered.clone(),
                    3..=6,
                )
                .build()?,
            scanout: builder
                .add_preference_tranche(
                    scanout.dev_id(),
                    TrancheFlags::Scanout,
                    plane_formats,
                    4..=6,
                )
                .add_preference_tranche(scanout.dev_id(), TrancheFlags::Sampling, rendered, 4..=6)
                .build()?,
        })
    }
}

/// Formats the CRTC's primary or overlay planes take, among `drawable`.
fn scanout_formats(surface: &DrmSurface, drawable: &FormatSet) -> FormatSet {
    let planes = surface.planes();
    planes
        .primary
        .iter()
        .chain(&planes.overlay)
        .flat_map(|plane| plane.formats.iter())
        .filter(|format| drawable.contains(format))
        .copied()
        .collect()
}

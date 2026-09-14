//! The delegate type, the handler trait, and what the renderer reads.

use std::sync::{Arc, Mutex};

use smithay::wayland::compositor::{Cacheable, SurfaceData, with_states};
use wayland_protocols::wp::color_management::v1::server::{
    wp_color_management_output_v1::WpColorManagementOutputV1,
    wp_image_description_info_v1::WpImageDescriptionInfoV1 as InfoObject,
    wp_color_management_surface_feedback_v1::WpColorManagementSurfaceFeedbackV1,
    wp_color_management_surface_v1::WpColorManagementSurfaceV1,
    wp_color_manager_v1::WpColorManagerV1,
    wp_image_description_creator_icc_v1::WpImageDescriptionCreatorIccV1,
    wp_image_description_creator_params_v1::WpImageDescriptionCreatorParamsV1,
    wp_image_description_v1::WpImageDescriptionV1,
};
use wayland_server::{
    Client, Dispatch, DisplayHandle, GlobalDispatch, Resource, Weak, backend::GlobalId,
    protocol::{wl_output::WlOutput, wl_surface::WlSurface},
};

use super::{
    capabilities::RenderIntent,
    record::{DescriptionId, DescriptionKind, IccData, ImageDescription, ParametricDescription},
    registry::Registry,
};

/// The protocol version this module speaks.
pub const VERSION: u32 = 3;

/// What a surface has committed: a description and how to render into it.
#[derive(Debug, Clone)]
pub struct SurfaceColor {
    pub description: Arc<ImageDescription>,
    pub intent: RenderIntent,
}

/// Double-buffered per-surface state, applied on commit like every other
/// surface property.
#[derive(Debug, Clone, Default)]
pub struct SurfaceColorCachedState {
    /// `None` means the surface has no description, which the protocol
    /// defines as sRGB primaries with a `gamma22` curve at reference white.
    pub color: Option<SurfaceColor>,
}

impl Cacheable for SurfaceColorCachedState {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

/// A surface's committed colour, for the renderer.
///
/// The single entry point out of this module into the render path: everything
/// else here is protocol bookkeeping.
pub fn surface_color(states: &SurfaceData) -> Option<SurfaceColor> {
    states
        .cached_state
        .get::<SurfaceColorCachedState>()
        .current()
        .color
        .clone()
}

/// Reads a surface's colour without holding a `SurfaceData`.
pub fn color_of(surface: &WlSurface) -> Option<SurfaceColor> {
    with_states(surface, surface_color)
}

pub trait ColorManagementHandler {
    fn color_management_state(&mut self) -> &mut ColorManagementState;

    /// The description backing an output, or `None` if it has gone.
    fn output_image_description(&mut self, output: &WlOutput) -> Option<Arc<ImageDescription>>;

    /// The description a surface should ideally render into.
    ///
    /// Usually the output's, but not always: an HDR output prefers BT.2100 PQ
    /// because that is what it can scan out with no conversion at all.
    fn preferred_image_description(&mut self, surface: &WlSurface)
    -> Option<Arc<ImageDescription>>;

    /// Whether this ICC profile is one the renderer can actually honour.
    ///
    /// The protocol layer has already checked the header; this is the
    /// compositor's own parser having the final say.
    fn accept_icc(&mut self, icc: &IccData) -> Result<(), std::borrow::Cow<'static, str>>;

    /// A veto on a parametric combination the renderer cannot produce.
    fn accept_parametric(
        &mut self,
        _description: &ParametricDescription,
    ) -> Result<(), std::borrow::Cow<'static, str>> {
        Ok(())
    }

    /// A surface's committed colour changed, so its rendering is stale.
    fn surface_color_changed(&mut self, _surface: &WlSurface) {}

    /// Send `description`'s information on `object`, on a later turn of the
    /// event loop.
    ///
    /// It cannot be sent inline. `wp_image_description_info_v1.done` is a
    /// *destructor* event, and wayland-rs only attaches an object's user data
    /// after the request that created it has returned — so destroying the
    /// object inside that request leaves the backend reaching for something
    /// that is already gone, and it panics.
    ///
    /// The compositor implements this with whatever idle mechanism its event
    /// loop offers; [`send_image_description_info`] is the body to run.
    fn defer_image_description_info(
        &mut self,
        object: InfoObject,
        description: Arc<ImageDescription>,
    );
}

/// Which clients may see the manager global.
pub struct ColorManagementGlobalData {
    pub(super) filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Feedback objects watching one surface.
struct Feedback {
    surface: Weak<WlSurface>,
    object: WpColorManagementSurfaceFeedbackV1,
    /// What this object was last told, so an unchanged preference sends
    /// nothing — clients rebuild their pipeline on every one of these.
    last_sent: Option<DescriptionId>,
}

pub struct ColorManagementState {
    global: GlobalId,
    registry: Registry,
    feedback: Vec<Feedback>,
    /// Outputs whose description clients are watching.
    outputs: Vec<(WlOutput, WpColorManagementOutputV1)>,
}

impl ColorManagementState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<WpColorManagerV1, ColorManagementGlobalData>
            + Dispatch<WpColorManagerV1, ()>
            + ColorManagementHandler
            + 'static,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        Self {
            global: display.create_global::<D, WpColorManagerV1, _>(
                VERSION,
                ColorManagementGlobalData {
                    filter: Box::new(filter),
                },
            ),
            registry: Registry::new(),
            feedback: Vec::new(),
            outputs: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Interns a description the compositor itself authored.
    ///
    /// Used for an output's own description, so a client comparing its
    /// surface's description against the output's sees the same identity when
    /// they genuinely match.
    pub fn intern(&mut self, kind: DescriptionKind) -> Arc<ImageDescription> {
        self.registry.intern(kind)
    }

    pub(super) fn watch_output(&mut self, output: &WlOutput, object: WpColorManagementOutputV1) {
        self.outputs.push((output.clone(), object));
    }

    pub(super) fn forget_output(&mut self, object: &WpColorManagementOutputV1) {
        self.outputs.retain(|(_, existing)| existing != object);
    }

    pub(super) fn watch_feedback(
        &mut self,
        surface: &WlSurface,
        object: WpColorManagementSurfaceFeedbackV1,
    ) {
        self.feedback.push(Feedback {
            surface: surface.downgrade(),
            object,
            last_sent: None,
        });
    }

    pub(super) fn forget_feedback(&mut self, object: &WpColorManagementSurfaceFeedbackV1) {
        self.feedback.retain(|feedback| feedback.object != *object);
    }

    /// Tells everyone watching an output that its colours changed.
    ///
    /// The `wl_output.done` that has to follow is the compositor's to send,
    /// because it owns the output's other properties too.
    pub fn output_description_changed(&mut self, output: &WlOutput) {
        for (watched, object) in &self.outputs {
            if watched == output {
                object.image_description_changed();
            }
        }
    }

    /// Re-evaluates every surface's preferred description.
    ///
    /// Sends only where the identity actually changed: a client rebuilds its
    /// render pipeline on each of these, so a spurious one is expensive.
    pub fn refresh_preferred<D>(state: &mut D)
    where
        D: ColorManagementHandler,
    {
        let watched: Vec<(WlSurface, WpColorManagementSurfaceFeedbackV1)> = state
            .color_management_state()
            .feedback
            .iter()
            .filter_map(|feedback| {
                Some((feedback.surface.upgrade().ok()?, feedback.object.clone()))
            })
            .collect();

        for (surface, object) in watched {
            let Some(preferred) = state.preferred_image_description(&surface) else {
                continue;
            };

            let this = state.color_management_state();
            let Some(feedback) = this
                .feedback
                .iter_mut()
                .find(|feedback| feedback.object == object)
            else {
                continue;
            };
            if feedback.last_sent == Some(preferred.id()) {
                continue;
            }
            feedback.last_sent = Some(preferred.id());

            // Version 1 has only the deprecated event, which carries a 32-bit
            // identity; from 2 the wider one supersedes it.
            if object.version() >= 2 {
                let (high, low) = preferred.id().halves();
                object.preferred_changed2(high, low);
            } else {
                object.preferred_changed(preferred.id().truncated());
            }
        }
    }

    /// Drops objects whose surface has gone.
    pub fn cleanup(&mut self) {
        self.feedback
            .retain(|feedback| feedback.surface.upgrade().is_ok());
    }
}

/// User data of a `wp_color_management_output_v1`.
#[derive(Debug)]
pub struct OutputData {
    pub output: WlOutput,
}

/// User data of a `wp_color_management_surface_v1`.
#[derive(Debug)]
pub struct SurfaceData_ {
    pub surface: WlSurface,
}

/// User data of a `wp_color_management_surface_feedback_v1`.
#[derive(Debug)]
pub struct FeedbackData {
    pub surface: WlSurface,
}

/// User data of a `wp_image_description_v1`.
#[derive(Debug)]
pub struct DescriptionData {
    pub state: Mutex<DescriptionState>,
    /// Whether `get_information` may be answered.
    ///
    /// A property of the *object*, not of the description: the same colours
    /// reached through `get_preferred` may be read back, and through a creator
    /// may not.
    pub allows_info: bool,
}

/// A description object's lifecycle.
#[derive(Debug, Default)]
pub enum DescriptionState {
    /// Created, but neither `ready` nor `failed` has been sent yet.
    #[default]
    Pending,
    Ready(Arc<ImageDescription>),
    Failed,
}

/// User data of a `wp_image_description_info_v1`.
#[derive(Debug)]
pub struct InfoData;

/// User data of a `wp_image_description_reference_v1`.
#[derive(Debug)]
pub struct ReferenceData {
    pub description: Arc<ImageDescription>,
    pub allows_info: bool,
}

/// The dispatch bounds every impl in this protocol shares.
pub trait ColorDispatch:
    GlobalDispatch<WpColorManagerV1, ColorManagementGlobalData>
    + Dispatch<WpColorManagerV1, ()>
    + Dispatch<WpColorManagementOutputV1, OutputData>
    + Dispatch<WpColorManagementSurfaceV1, SurfaceData_>
    + Dispatch<WpColorManagementSurfaceFeedbackV1, FeedbackData>
    + Dispatch<WpImageDescriptionCreatorParamsV1, super::creator::ParamsData>
    + Dispatch<WpImageDescriptionCreatorIccV1, super::creator::IccCreatorData>
    + Dispatch<WpImageDescriptionV1, DescriptionData>
    + Dispatch<InfoObject, InfoData>
    + ColorManagementHandler
    + 'static
{
}

impl<D> ColorDispatch for D where
    D: GlobalDispatch<WpColorManagerV1, ColorManagementGlobalData>
        + Dispatch<WpColorManagerV1, ()>
        + Dispatch<WpColorManagementOutputV1, OutputData>
        + Dispatch<WpColorManagementSurfaceV1, SurfaceData_>
        + Dispatch<WpColorManagementSurfaceFeedbackV1, FeedbackData>
        + Dispatch<WpImageDescriptionCreatorParamsV1, super::creator::ParamsData>
        + Dispatch<WpImageDescriptionCreatorIccV1, super::creator::IccCreatorData>
        + Dispatch<WpImageDescriptionV1, DescriptionData>
        + Dispatch<InfoObject, InfoData>
        + ColorManagementHandler
        + 'static
{
}

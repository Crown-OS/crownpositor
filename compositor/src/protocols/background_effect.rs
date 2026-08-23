//! Server-side `ext-background-effect-v1` (staging).
//!
//! Lets a client mark a region of its surface whose *background* — whatever
//! the compositor composites behind it — should be blurred. The protocol is
//! only bookkeeping: this module validates requests and leaves a
//! double-buffered [`BlurRegionCachedState`] on the surface, which the
//! renderer reads at element-build time. No rendering happens here, keeping
//! the protocol layer fully decoupled from the graphics backends.
//!
//! Written the way smithay writes its own protocol modules (see
//! `smithay::wayland::alpha_modifier`): the state's `Dispatch` impls are
//! generic over the compositor data `D`, and `handlers::background_effect`
//! delegates to them.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use smithay::{
    reexports::wayland_protocols::ext::background_effect::v1::server::{
        ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
        ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
    },
    utils::{Logical, Rectangle, Size},
    wayland::compositor::{
        Cacheable, RectangleKind, RegionAttributes, get_region_attributes, with_states,
    },
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
    backend::GlobalId, protocol::wl_surface::WlSurface,
};

/// The blur region a client has committed for its surface.
///
/// Double-buffered through smithay's cached-state machinery, so it applies
/// atomically with the `wl_surface.commit` it rode in on — a client resizing
/// its window and its blur region together never shows one without the other.
#[derive(Debug, Default, Clone)]
pub struct BlurRegionCachedState {
    /// `None` means no blur. The protocol's "empty region" initial state is
    /// represented the same way, because an empty region blurs nothing.
    region: Option<RegionAttributes>,
}

impl BlurRegionCachedState {
    /// The committed blur region's bounding rectangle, clipped to the surface,
    /// in surface-local logical coordinates. `None` when nothing should blur.
    ///
    /// A bounding box rather than the exact rect list: windows asking for
    /// anything but "all of me" are rare, and one backdrop element per window
    /// keeps damage tracking and the corner mask simple. The exact-region
    /// upgrade can live entirely inside this method when it matters.
    pub fn blur_bounds(&self, surface_size: Size<i32, Logical>) -> Option<Rectangle<i32, Logical>> {
        let region = self.region.as_ref()?;

        let mut bounds: Option<Rectangle<i32, Logical>> = None;
        for (kind, rect) in &region.rects {
            // Subtractions only matter for exact-region rendering; for a
            // bounding box the additions define the extent.
            if matches!(kind, RectangleKind::Add) {
                bounds = Some(match bounds {
                    Some(prev) => prev.merge(*rect),
                    None => *rect,
                });
            }
        }

        bounds?.intersection(Rectangle::from_size(surface_size))
    }
}

impl Cacheable for BlurRegionCachedState {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

/// Per-surface marker enforcing the "one effect object per surface" rule.
/// Lives in the surface's `data_map`, which outlives any one effect object.
#[derive(Debug, Default)]
struct BackgroundEffectAttached(AtomicBool);

/// User data of an [`ExtBackgroundEffectSurfaceV1`] object.
///
/// Holds the surface weakly: the protocol says the object goes inert when its
/// surface dies, and a strong handle here would keep the surface alive instead.
#[derive(Debug)]
pub struct BackgroundEffectSurfaceData {
    surface: Mutex<Weak<WlSurface>>,
}

impl BackgroundEffectSurfaceData {
    fn new(surface: &WlSurface) -> Self {
        Self {
            surface: Mutex::new(surface.downgrade()),
        }
    }

    fn wl_surface(&self) -> Option<WlSurface> {
        self.surface
            .lock()
            .ok()
            .and_then(|weak| weak.upgrade().ok())
    }
}

/// Delegate type for the [`ExtBackgroundEffectManagerV1`] global.
#[derive(Debug)]
pub struct BackgroundEffectState {
    global: GlobalId,
    capabilities: Capability,
}

impl BackgroundEffectState {
    /// Registers the global. `capabilities` is what gets advertised on bind —
    /// pass `Capability::empty()` if the renderer's blur shaders failed to
    /// compile, and clients will know not to ask.
    pub fn new<D>(display: &DisplayHandle, capabilities: Capability) -> Self
    where
        D: GlobalDispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
            + Dispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
            + Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceData>
            + 'static,
    {
        let global = display.create_global::<D, ExtBackgroundEffectManagerV1, _>(
            1,
            BackgroundEffectGlobalData { capabilities },
        );
        Self {
            global,
            capabilities,
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    pub fn capabilities(&self) -> Capability {
        self.capabilities
    }
}

/// What a bound manager needs to know; carried on the global and every
/// manager resource, because the capabilities event is per-bind.
#[derive(Debug, Clone, Copy)]
pub struct BackgroundEffectGlobalData {
    capabilities: Capability,
}

impl<D> GlobalDispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData, D>
    for BackgroundEffectState
where
    D: GlobalDispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
        + Dispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
        + Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceData>
        + 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        global_data: &BackgroundEffectGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, *global_data);
        // "The capabilities are send when the global is bound" — and they only
        // change with a renderer swap, which for us means a restart.
        manager.capabilities(global_data.capabilities);
    }
}

impl<D> Dispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData, D>
    for BackgroundEffectState
where
    D: Dispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
        + Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceData>
        + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        manager: &ExtBackgroundEffectManagerV1,
        request: ext_background_effect_manager_v1::Request,
        _data: &BackgroundEffectGlobalData,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_background_effect_manager_v1::Request::GetBackgroundEffect { id, surface } => {
                let already_attached = with_states(&surface, |states| {
                    states
                        .data_map
                        .insert_if_missing_threadsafe(BackgroundEffectAttached::default);
                    states
                        .data_map
                        .get::<BackgroundEffectAttached>()
                        .map(|marker| marker.0.swap(true, Ordering::AcqRel))
                        .unwrap_or(false)
                });

                if already_attached {
                    manager.post_error(
                        ext_background_effect_manager_v1::Error::BackgroundEffectExists,
                        "the surface already has a background effect object",
                    );
                    return;
                }

                data_init.init(id, BackgroundEffectSurfaceData::new(&surface));
            }
            ext_background_effect_manager_v1::Request::Destroy => {
                // Objects created through the manager outlive it by design.
            }
            _ => {}
        }
    }
}

impl<D> Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceData, D>
    for BackgroundEffectState
where
    D: Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceData> + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        effect: &ExtBackgroundEffectSurfaceV1,
        request: ext_background_effect_surface_v1::Request,
        data: &BackgroundEffectSurfaceData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_background_effect_surface_v1::Request::SetBlurRegion { region } => {
                let Some(surface) = data.wl_surface() else {
                    effect.post_error(
                        ext_background_effect_surface_v1::Error::SurfaceDestroyed,
                        "the associated surface has been destroyed",
                    );
                    return;
                };

                // Copy semantics: the region's rects are read out now, so the
                // client can destroy the wl_region right away.
                let pending = region.as_ref().map(get_region_attributes);
                with_states(&surface, |states| {
                    states
                        .cached_state
                        .get::<BlurRegionCachedState>()
                        .pending()
                        .region = pending;
                });
            }
            ext_background_effect_surface_v1::Request::Destroy => {
                // "The effect regions will be removed on the next commit":
                // clear the *pending* state and let the client's commit apply
                // it, exactly like any other double-buffered change.
                if let Some(surface) = data.wl_surface() {
                    with_states(&surface, |states| {
                        states
                            .cached_state
                            .get::<BlurRegionCachedState>()
                            .pending()
                            .region = None;
                    });
                }
            }
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut D,
        _client: wayland_server::backend::ClientId,
        _effect: &ExtBackgroundEffectSurfaceV1,
        data: &BackgroundEffectSurfaceData,
    ) {
        // Free the slot so the surface can get a new effect object; covers
        // both explicit destroy and the client disconnecting.
        if let Some(surface) = data.wl_surface() {
            with_states(&surface, |states| {
                if let Some(marker) = states.data_map.get::<BackgroundEffectAttached>() {
                    marker.0.store(false, Ordering::Release);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region_of(rects: &[(RectangleKind, Rectangle<i32, Logical>)]) -> BlurRegionCachedState {
        BlurRegionCachedState {
            region: Some(RegionAttributes {
                rects: rects.to_vec(),
            }),
        }
    }

    #[test]
    fn no_region_means_no_blur() {
        let state = BlurRegionCachedState::default();
        assert_eq!(state.blur_bounds(Size::from((100, 100))), None);
    }

    #[test]
    fn empty_region_means_no_blur() {
        // The protocol's initial state: an effect object exists, but its
        // region is empty — nothing should blur.
        let state = region_of(&[]);
        assert_eq!(state.blur_bounds(Size::from((100, 100))), None);
    }

    #[test]
    fn bounds_are_clipped_to_the_surface() {
        let state = region_of(&[(
            RectangleKind::Add,
            Rectangle::new((-50, -50).into(), (400, 400).into()),
        )]);
        assert_eq!(
            state.blur_bounds(Size::from((100, 100))),
            Some(Rectangle::new((0, 0).into(), (100, 100).into()))
        );
    }

    #[test]
    fn bounds_merge_multiple_adds() {
        let state = region_of(&[
            (
                RectangleKind::Add,
                Rectangle::new((0, 0).into(), (10, 10).into()),
            ),
            (
                RectangleKind::Add,
                Rectangle::new((90, 90).into(), (10, 10).into()),
            ),
        ]);
        assert_eq!(
            state.blur_bounds(Size::from((100, 100))),
            Some(Rectangle::new((0, 0).into(), (100, 100).into()))
        );
    }

    #[test]
    fn subtract_only_region_blurs_nothing() {
        let state = region_of(&[(
            RectangleKind::Subtract,
            Rectangle::new((0, 0).into(), (10, 10).into()),
        )]);
        assert_eq!(state.blur_bounds(Size::from((100, 100))), None);
    }
}

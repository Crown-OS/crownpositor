//! Server-side `ext-background-effect-v1` (staging).
//!
//! Lets a client mark a region of its surface whose *background* — whatever
//! the compositor composites behind it — should be blurred. The protocol is
//! only bookkeeping: this module validates requests, applies the
//! double-buffered region at commit, and leaves the result on the surface as
//! a list of disjoint rectangles the renderer reads at element-build time. No
//! rendering happens here, keeping the protocol layer fully decoupled from the
//! graphics backends.
//!
//! Written the way smithay writes its own protocol modules (see
//! `smithay::wayland::alpha_modifier`): a handler trait the compositor
//! implements, `Dispatch` impls generic over the compositor data `D`, and the
//! compositor supplying the delegation glue.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use smithay::{
    reexports::wayland_protocols::ext::background_effect::v1::server::{
        ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
        ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
    },
    utils::{Logical, Rectangle},
    wayland::compositor::{
        Cacheable, RectangleKind, RegionAttributes, SurfaceData, add_post_commit_hook,
        get_region_attributes, with_states,
    },
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
    backend::GlobalId, protocol::wl_surface::WlSurface,
};

use crate::region::region_rects;

pub use ext_background_effect_manager_v1::Capability;

/// How many rectangles of one surface's blur region the compositor honours.
///
/// A `wl_region` can be arbitrarily intricate, and every rectangle that
/// survives here costs the renderer a draw call in every frame the surface is
/// on screen. "The blur algorithm is subject to compositor policies", and this
/// is one of them; real interfaces ask for a handful at most.
const MAX_REGION_RECTS: usize = 64;

/// What the compositor has to provide for this protocol to be delegated to
/// [`BackgroundEffectState`].
pub trait BackgroundEffectHandler {
    fn background_effect_state(&mut self) -> &mut BackgroundEffectState;
}

/// The blur region a client has committed for its surface.
///
/// Double-buffered through smithay's cached-state machinery, so it applies
/// atomically with the `wl_surface.commit` it rode in on — a client resizing
/// its window and its blur region together never shows one without the other.
#[derive(Debug, Default, Clone)]
pub struct BlurRegionCachedState {
    /// `None` means "no blur region", which is distinct from a region that is
    /// present but empty only in that the latter still holds the surface's one
    /// effect slot; both blur nothing.
    region: Option<RegionAttributes>,
}

impl Cacheable for BlurRegionCachedState {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

/// A surface's committed blur region, ready to draw.
///
/// The rectangles are disjoint and in surface-local logical coordinates,
/// *un*clipped: the protocol has the compositor clip to the surface size, and
/// the renderer is where that size is actually known, so it clips as it places
/// each rect.
#[derive(Debug, Clone)]
pub struct BlurRegion {
    /// Shared rather than cloned per frame: this list changes about as often
    /// as a client changes its mind, and gets read every frame.
    pub rects: Arc<Vec<Rectangle<i32, Logical>>>,
    /// Bumped whenever `rects` changes. The renderer folds this into its
    /// backdrop elements' commit counters, which is what makes a region change
    /// repaint rather than sit there until something else damages the window.
    pub generation: u32,
}

/// Reads a surface's committed blur region. `None` when the client has not set
/// one, or set it and then let the effect object go.
///
/// Cheap: the rectangles were computed at commit time, and this clones two
/// words.
pub fn blur_region(states: &SurfaceData) -> Option<BlurRegion> {
    let cache = states.data_map.get::<BlurRegionCache>()?;
    let cached = cache.0.lock().unwrap();
    Some(BlurRegion {
        rects: cached.rects.clone()?,
        generation: cached.generation,
    })
}

/// Per-surface derived state, living in the surface's `data_map` because it
/// has to outlive the effect object that produced it — the object can go away
/// while its region stays committed until the next commit.
#[derive(Debug, Default)]
struct BlurRegionCache(Mutex<CachedRegion>);

#[derive(Debug, Default)]
struct CachedRegion {
    /// The region as committed, kept only to tell "the client re-set the same
    /// region" from "the region changed" without redoing the decomposition on
    /// every commit — which, for a surface that also draws video, is every
    /// frame.
    region: Option<RegionAttributes>,
    /// `region`, decomposed into disjoint rectangles.
    rects: Option<Arc<Vec<Rectangle<i32, Logical>>>>,
    generation: u32,
}

/// Applies the double-buffered region, which
/// [`add_post_commit_hook`]'s contract says is this module's job to do at
/// commit time.
fn apply_committed_region(states: &SurfaceData) {
    // The cached state only exists once a client has touched this protocol;
    // `has` keeps an unrelated surface's commit from allocating one.
    if !states.cached_state.has::<BlurRegionCachedState>() {
        return;
    }
    let committed = states
        .cached_state
        .get::<BlurRegionCachedState>()
        .current()
        .region
        .clone();

    let cache = states
        .data_map
        .get_or_insert_threadsafe(BlurRegionCache::default);
    let mut cached = cache.0.lock().unwrap();

    if same_region(cached.region.as_ref(), committed.as_ref()) {
        return;
    }

    cached.rects = committed.as_ref().map(|region| {
        let mut rects = Vec::new();
        region_rects(region, &mut rects);
        if rects.len() > MAX_REGION_RECTS {
            tracing::debug!(
                asked = rects.len(),
                limit = MAX_REGION_RECTS,
                "blur region is more intricate than we draw; truncating"
            );
            rects.truncate(MAX_REGION_RECTS);
        }
        Arc::new(rects)
    });
    cached.region = committed;
    // Wrapping is fine: the renderer only ever compares for equality, and a
    // region would have to change four billion times between two frames to
    // land back on the value it started at.
    cached.generation = cached.generation.wrapping_add(1);
}

/// `RegionAttributes` is not `PartialEq` upstream, and the derived comparison
/// would be wrong anyway — two regions with the same rects in a different
/// order describe different sets, so this is a literal list comparison.
fn same_region(a: Option<&RegionAttributes>, b: Option<&RegionAttributes>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };

    a.rects.len() == b.rects.len()
        && std::iter::zip(&a.rects, &b.rects).all(|((a_kind, a_rect), (b_kind, b_rect))| {
            a_rect == b_rect
                && matches!(
                    (a_kind, b_kind),
                    (RectangleKind::Add, RectangleKind::Add)
                        | (RectangleKind::Subtract, RectangleKind::Subtract)
                )
        })
}

/// Per-surface marker enforcing the "one effect object per surface" rule, plus
/// the "hook is registered" latch. Lives in the surface's `data_map`, which
/// outlives any one effect object.
#[derive(Debug, Default)]
struct SurfaceSlot {
    taken: AtomicBool,
    hooked: AtomicBool,
}

/// User data of an [`ExtBackgroundEffectSurfaceV1`] object.
#[derive(Debug)]
pub struct BackgroundEffectSurfaceData {
    /// Holds the surface weakly: the protocol says the object goes inert when
    /// its surface dies, and a strong handle here would keep the surface alive
    /// instead.
    ///
    /// `None` marks an object that lost the race for its surface's one effect
    /// slot. Such an object exists only so the protocol error has something
    /// live to be posted on, and must never touch the surface — least of all
    /// release the slot its rightful owner is holding.
    surface: Option<Weak<WlSurface>>,
}

impl BackgroundEffectSurfaceData {
    fn new(surface: &WlSurface) -> Self {
        Self {
            surface: Some(surface.downgrade()),
        }
    }

    fn duplicate() -> Self {
        Self { surface: None }
    }

    fn wl_surface(&self) -> Option<WlSurface> {
        self.surface.as_ref()?.upgrade().ok()
    }
}

/// Delegate type for the [`ExtBackgroundEffectManagerV1`] global.
#[derive(Debug)]
pub struct BackgroundEffectState {
    global: GlobalId,
    capabilities: Capability,
    /// Every manager a client currently holds, so a capability change can be
    /// announced — the protocol promises the event "every time they change",
    /// not just on bind.
    managers: Vec<Weak<ExtBackgroundEffectManagerV1>>,
}

impl BackgroundEffectState {
    /// Registers the global. `capabilities` is what gets advertised on bind —
    /// pass [`Capability::empty()`] if the renderer's blur shaders failed to
    /// compile, and clients will know not to ask.
    pub fn new<D>(display: &DisplayHandle, capabilities: Capability) -> Self
    where
        D: GlobalDispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
            + Dispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
            + Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceData>
            + BackgroundEffectHandler
            + 'static,
    {
        let global = display
            .create_global::<D, ExtBackgroundEffectManagerV1, _>(1, BackgroundEffectGlobalData);
        Self {
            global,
            capabilities,
            managers: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    pub fn capabilities(&self) -> Capability {
        self.capabilities
    }

    /// Announces a new capability set to every bound manager.
    ///
    /// "Note that when the capability goes away, the corresponding effect is
    /// no longer applied by the compositor, even if it was set before" — so
    /// the renderer is free to stop drawing blur the moment this is called;
    /// committed regions stay committed and take effect again if the
    /// capability comes back.
    pub fn set_capabilities(&mut self, capabilities: Capability) {
        // Dead managers are pruned regardless, so a client that binds and
        // disconnects in a loop cannot grow this list without bound.
        self.managers.retain(|manager| manager.upgrade().is_ok());
        if self.capabilities == capabilities {
            return;
        }

        self.capabilities = capabilities;
        for manager in &self.managers {
            if let Ok(manager) = manager.upgrade() {
                manager.capabilities(capabilities);
            }
        }
    }
}

/// User data of the global and of every manager bound from it. Empty because
/// the capabilities are read from [`BackgroundEffectState`] on each bind
/// rather than snapshotted here, which is what lets them change afterwards.
#[derive(Debug, Clone, Copy)]
pub struct BackgroundEffectGlobalData;

impl<D> GlobalDispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData, D>
    for BackgroundEffectState
where
    D: GlobalDispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
        + Dispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
        + Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceData>
        + BackgroundEffectHandler
        + 'static,
{
    fn bind(
        state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        _global_data: &BackgroundEffectGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, BackgroundEffectGlobalData);

        let effect_state = state.background_effect_state();
        // "The capabilities are send when the global is bound, and every time
        // they change."
        manager.capabilities(effect_state.capabilities);
        // Pruned here as well as on a capability change, so a client that binds
        // and drops the global in a loop cannot grow the list without bound.
        effect_state
            .managers
            .retain(|manager| manager.upgrade().is_ok());
        effect_state.managers.push(manager.downgrade());
    }
}

impl<D> Dispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData, D>
    for BackgroundEffectState
where
    D: Dispatch<ExtBackgroundEffectManagerV1, BackgroundEffectGlobalData>
        + Dispatch<ExtBackgroundEffectSurfaceV1, BackgroundEffectSurfaceData>
        + BackgroundEffectHandler
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
                let taken = with_states(&surface, |states| {
                    let slot = states
                        .data_map
                        .get_or_insert_threadsafe(SurfaceSlot::default);
                    slot.taken.swap(true, Ordering::AcqRel)
                });

                if taken {
                    // The new object is initialised *before* the error is
                    // posted: wayland-rs panics if a `new_id` request returns
                    // without initialising its object, so bailing out early
                    // would turn a client's protocol violation into the
                    // compositor going down with it.
                    data_init.init(id, BackgroundEffectSurfaceData::duplicate());
                    manager.post_error(
                        ext_background_effect_manager_v1::Error::BackgroundEffectExists,
                        "the surface already has a background effect object",
                    );
                    return;
                }

                // Registered once per surface and never removed: the hook is
                // what applies the double-buffered region, and it has to keep
                // running after the effect object is destroyed to apply the
                // clearing that destruction schedules. Claimed only once the
                // slot is ours, so the error path above cannot latch it
                // without a hook behind it.
                let hooked = with_states(&surface, |states| {
                    let slot = states
                        .data_map
                        .get_or_insert_threadsafe(SurfaceSlot::default);
                    slot.hooked.swap(true, Ordering::AcqRel)
                });
                if !hooked {
                    add_post_commit_hook::<D, _>(&surface, |_state, _dh, surface| {
                        with_states(surface, apply_committed_region);
                    });
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
                set_pending_region(&surface, pending);
            }
            ext_background_effect_surface_v1::Request::Destroy => {
                // "The effect regions will be removed on the next commit":
                // clear the *pending* state and let the client's commit apply
                // it, exactly like any other double-buffered change.
                if let Some(surface) = data.wl_surface() {
                    set_pending_region(&surface, None);
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
        // both explicit destroy and the client disconnecting. A duplicate
        // object holds no surface and so releases nothing.
        let Some(surface) = data.wl_surface() else {
            return;
        };
        with_states(&surface, |states| {
            if let Some(slot) = states.data_map.get::<SurfaceSlot>() {
                slot.taken.store(false, Ordering::Release);
            }
        });
    }
}

fn set_pending_region(surface: &WlSurface, region: Option<RegionAttributes>) {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<BlurRegionCachedState>()
            .pending()
            .region = region;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attributes(rects: &[(RectangleKind, Rectangle<i32, Logical>)]) -> RegionAttributes {
        RegionAttributes {
            rects: rects.to_vec(),
        }
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn absence_and_emptiness_are_different_regions() {
        // Both blur nothing, but only one of them is a change when committed
        // over the other — so the comparison the commit hook uses has to tell
        // them apart or a client clearing its region never repaints.
        assert!(!same_region(None, Some(&attributes(&[]))));
        assert!(same_region(None, None));
        assert!(same_region(Some(&attributes(&[])), Some(&attributes(&[]))));
    }

    #[test]
    fn order_is_part_of_a_region() {
        // Add-then-subtract is a hole; subtract-then-add is a filled rect.
        let hole = attributes(&[
            (RectangleKind::Add, rect(0, 0, 30, 30)),
            (RectangleKind::Subtract, rect(10, 10, 10, 10)),
        ]);
        let filled = attributes(&[
            (RectangleKind::Subtract, rect(10, 10, 10, 10)),
            (RectangleKind::Add, rect(0, 0, 30, 30)),
        ]);
        assert!(!same_region(Some(&hole), Some(&filled)));
    }

    #[test]
    fn identical_regions_compare_equal() {
        let one = attributes(&[
            (RectangleKind::Add, rect(0, 0, 30, 30)),
            (RectangleKind::Subtract, rect(10, 10, 10, 10)),
        ]);
        let two = one.clone();
        assert!(same_region(Some(&one), Some(&two)));

        // A moved subtraction is a different region even though the list has
        // the same shape.
        let moved = attributes(&[
            (RectangleKind::Add, rect(0, 0, 30, 30)),
            (RectangleKind::Subtract, rect(11, 10, 10, 10)),
        ]);
        assert!(!same_region(Some(&one), Some(&moved)));
    }
}

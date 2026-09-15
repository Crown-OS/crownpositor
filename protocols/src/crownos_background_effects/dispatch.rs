//! Request routing for `crownos_background_effects`.
//!
//! Two globals, four interfaces, one state type. Written the way smithay writes
//! its own protocol modules: every impl is generic over the compositor data
//! `D`, so nothing here can reach into a compositor, and the compositor supplies
//! the delegation glue.

use std::sync::atomic::Ordering;

use smithay::{
    utils::{Point, Rectangle, Size},
    wayland::compositor::{add_post_commit_hook, with_states},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    protocol::wl_surface::WlSurface,
};

use crownos_protocols::background_effects::v1::server::{
    crownos_background_effect_manager_v1::{self, CrownosBackgroundEffectManagerV1},
    crownos_background_effect_surface_v1::{self, CrownosBackgroundEffectSurfaceV1},
    crownos_shape_manager_v1::{self, CrownosShapeManagerV1},
    crownos_shape_v1::{self, CrownosShapeV1},
};

use crate::crownos_background_effects::{
    BackgroundEffectsHandler, BackgroundEffectsState, EffectSurfaceData, GlobalData,
    color::Argb,
    effects::{Blur, Effects, Shadow, apply_committed},
    shape::{RoundedRect, ShapeData},
};

/// Everything the compositor's data type has to satisfy for the whole protocol
/// to be delegated to [`BackgroundEffectsState`], in one bound.
///
/// Spelled once here rather than repeated across six impls: each of them needs
/// every other interface in scope, because any of them may create one.
pub trait BackgroundEffectsData:
    GlobalDispatch<CrownosShapeManagerV1, GlobalData>
    + GlobalDispatch<CrownosBackgroundEffectManagerV1, GlobalData>
    + Dispatch<CrownosShapeManagerV1, GlobalData>
    + Dispatch<CrownosShapeV1, ShapeData>
    + Dispatch<CrownosBackgroundEffectManagerV1, GlobalData>
    + Dispatch<CrownosBackgroundEffectSurfaceV1, EffectSurfaceData>
    + BackgroundEffectsHandler
    + 'static
{
}

impl<D> BackgroundEffectsData for D where
    D: GlobalDispatch<CrownosShapeManagerV1, GlobalData>
        + GlobalDispatch<CrownosBackgroundEffectManagerV1, GlobalData>
        + Dispatch<CrownosShapeManagerV1, GlobalData>
        + Dispatch<CrownosShapeV1, ShapeData>
        + Dispatch<CrownosBackgroundEffectManagerV1, GlobalData>
        + Dispatch<CrownosBackgroundEffectSurfaceV1, EffectSurfaceData>
        + BackgroundEffectsHandler
        + 'static
{
}

// --- shapes ---------------------------------------------------------------

impl<D: BackgroundEffectsData> GlobalDispatch<CrownosShapeManagerV1, GlobalData, D>
    for BackgroundEffectsState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosShapeManagerV1>,
        _global_data: &GlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, GlobalData);
    }
}

impl<D: BackgroundEffectsData> Dispatch<CrownosShapeManagerV1, GlobalData, D>
    for BackgroundEffectsState
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _manager: &CrownosShapeManagerV1,
        request: crownos_shape_manager_v1::Request,
        _data: &GlobalData,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            crownos_shape_manager_v1::Request::CreateShape { id } => {
                data_init.init(id, ShapeData::default());
            }
            // "Shapes created through this manager are unaffected."
            crownos_shape_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl<D: BackgroundEffectsData> Dispatch<CrownosShapeV1, ShapeData, D> for BackgroundEffectsState {
    fn request(
        _state: &mut D,
        _client: &Client,
        shape: &CrownosShapeV1,
        request: crownos_shape_v1::Request,
        data: &ShapeData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            crownos_shape_v1::Request::AddRoundedRect {
                x,
                y,
                width,
                height,
                radius,
            } => {
                if width < 0 || height < 0 {
                    shape.post_error(
                        crownos_shape_v1::Error::InvalidSize,
                        "a primitive cannot have a negative width or height",
                    );
                    return;
                }
                let rect = Rectangle::new(Point::from((x, y)), Size::from((width, height)));
                data.push(RoundedRect::new(rect, radius));
            }
            crownos_shape_v1::Request::AddCircle {
                center_x,
                center_y,
                radius,
            } => data.push(RoundedRect::circle(center_x, center_y, radius)),
            crownos_shape_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

// --- effects --------------------------------------------------------------

impl<D: BackgroundEffectsData> GlobalDispatch<CrownosBackgroundEffectManagerV1, GlobalData, D>
    for BackgroundEffectsState
{
    fn bind(
        state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosBackgroundEffectManagerV1>,
        _global_data: &GlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, GlobalData);

        // "Sent when the manager is bound, and again every time the set
        // changes."
        let state = state.background_effects_state();
        manager.capabilities(state.capabilities);
        state.remember(&manager);
    }
}

impl<D: BackgroundEffectsData> Dispatch<CrownosBackgroundEffectManagerV1, GlobalData, D>
    for BackgroundEffectsState
{
    fn request(
        _state: &mut D,
        _client: &Client,
        manager: &CrownosBackgroundEffectManagerV1,
        request: crownos_background_effect_manager_v1::Request,
        _data: &GlobalData,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            crownos_background_effect_manager_v1::Request::GetEffectSurface { id, surface } => {
                let taken = with_states(&surface, |states| {
                    let slot = states
                        .data_map
                        .get_or_insert_threadsafe(EffectSlot::default);
                    slot.taken.swap(true, Ordering::AcqRel)
                });

                if taken {
                    // Initialised *before* the error is posted: wayland-rs
                    // panics if a `new_id` request returns without initialising
                    // its object, so bailing out early would turn a client's
                    // protocol violation into the compositor going down with
                    // it.
                    data_init.init(id, EffectSurfaceData::duplicate());
                    manager.post_error(
                        crownos_background_effect_manager_v1::Error::EffectExists,
                        "the surface already has a background effect object",
                    );
                    return;
                }

                // Registered once per surface and never removed: the hook is
                // what applies the double-buffered effects, and it has to keep
                // running after the effect object is destroyed to apply the
                // clearing that destruction schedules. Claimed only once the
                // slot is ours, so the error path above cannot latch it without
                // a hook behind it.
                let hooked = with_states(&surface, |states| {
                    let slot = states
                        .data_map
                        .get_or_insert_threadsafe(EffectSlot::default);
                    slot.hooked.swap(true, Ordering::AcqRel)
                });
                if !hooked {
                    add_post_commit_hook::<D, _>(&surface, |_state, _dh, surface| {
                        with_states(surface, apply_committed);
                    });
                }

                data_init.init(id, EffectSurfaceData::new(&surface));
            }
            // Objects created through the manager outlive it by design.
            crownos_background_effect_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl<D: BackgroundEffectsData> Dispatch<CrownosBackgroundEffectSurfaceV1, EffectSurfaceData, D>
    for BackgroundEffectsState
{
    fn request(
        _state: &mut D,
        _client: &Client,
        effect: &CrownosBackgroundEffectSurfaceV1,
        request: crownos_background_effect_surface_v1::Request,
        data: &EffectSurfaceData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // "The effects are removed on the next commit": clear the *pending*
        // state and let the client's commit apply it, exactly like any other
        // double-buffered change.
        if let crownos_background_effect_surface_v1::Request::Destroy = request {
            if let Some(surface) = data.wl_surface() {
                with_pending(&surface, |pending| *pending = Effects::default());
            }
            return;
        }

        let Some(surface) = data.wl_surface() else {
            effect.post_error(
                crownos_background_effect_surface_v1::Error::SurfaceDestroyed,
                "the associated surface has been destroyed",
            );
            return;
        };

        match request {
            crownos_background_effect_surface_v1::Request::SetCornerRadius { radius } => {
                with_pending(&surface, |pending| pending.corner_radius = radius);
            }
            crownos_background_effect_surface_v1::Request::SetBorder { width } => {
                with_pending(&surface, |pending| pending.border_width = width);
            }
            crownos_background_effect_surface_v1::Request::SetBlur {
                shape,
                blur_radius,
                tint_color,
                saturation,
            } => {
                if saturation < 0.0 {
                    effect.post_error(
                        crownos_background_effect_surface_v1::Error::InvalidSaturation,
                        "saturation cannot be negative",
                    );
                    return;
                }
                // Copy semantics: the shape is read out now, so the client can
                // destroy it or keep adding to it right away.
                let shape = shape.as_ref().and_then(snapshot);
                with_pending(&surface, |pending| {
                    // "A blur_radius of zero turns the blur off, and with it
                    // the rim, the tint and the saturation."
                    pending.blur = (blur_radius > 0).then_some(Blur {
                        shape,
                        radius: blur_radius,
                        tint: Argb(tint_color),
                        saturation,
                    });
                });
            }
            crownos_background_effect_surface_v1::Request::SetShadow {
                shape,
                shadow_radius,
                offset_x,
                offset_y,
                color,
            } => {
                let shape = shape.as_ref().and_then(snapshot);
                let color = Argb(color);
                with_pending(&surface, |pending| {
                    // A shadow with no radius or no opacity draws nothing, and
                    // an effect that draws nothing must not cost the renderer
                    // an element or the damage tracker a repaint.
                    pending.shadow =
                        (shadow_radius > 0 && !color.is_transparent()).then(|| Shadow {
                            shape,
                            radius: shadow_radius,
                            offset: Point::from((offset_x, offset_y)),
                            color,
                        });
                });
            }
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut D,
        _client: wayland_server::backend::ClientId,
        _effect: &CrownosBackgroundEffectSurfaceV1,
        data: &EffectSurfaceData,
    ) {
        // Free the slot so the surface can get a new effect object; covers both
        // explicit destroy and the client disconnecting. A duplicate object
        // holds no surface and so releases nothing.
        let Some(surface) = data.wl_surface() else {
            return;
        };
        with_states(&surface, |states| {
            if let Some(slot) = states.data_map.get::<EffectSlot>() {
                slot.taken.store(false, Ordering::Release);
            }
        });
    }
}

/// A shape's primitives, or `None` for a shape whose object has already gone
/// inert — which cannot happen through a well-behaved client, and must not be
/// read as "the whole surface" if it does.
fn snapshot(
    shape: &CrownosShapeV1,
) -> Option<crate::crownos_background_effects::shape::Primitives> {
    Some(shape.data::<ShapeData>()?.snapshot())
}

fn with_pending(surface: &WlSurface, change: impl FnOnce(&mut Effects)) {
    with_states(surface, |states| {
        change(states.cached_state.get::<Effects>().pending());
    });
}

/// Per-surface marker enforcing the "one effect object per surface" rule, plus
/// the "hook is registered" latch. Lives in the surface's `data_map`, which
/// outlives any one effect object.
#[derive(Debug, Default)]
struct EffectSlot {
    taken: std::sync::atomic::AtomicBool,
    hooked: std::sync::atomic::AtomicBool,
}

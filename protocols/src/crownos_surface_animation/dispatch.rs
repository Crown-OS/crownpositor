//! Request routing for `crownos_surface_animation_v1`.

use std::time::Duration;

use crownos_protocols::surface_animation::v1::server::{
    crownos_animated_surface_v1::{self, CrownosAnimatedSurfaceV1},
    crownos_surface_animation_manager_v1::{self, CrownosSurfaceAnimationManagerV1},
};
use motion::Spring;
use smithay::{
    utils::{Clock, Monotonic},
    wayland::compositor::{SUBSURFACE_ROLE, add_post_commit_hook, get_role, with_states},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId, protocol::wl_surface::WlSurface,
};

use super::{
    AnimatedSurfaceData, SurfaceAnimationHandler, SurfaceAnimationState,
    animations::send,
    pending::{PendingMotion, PropertyChange},
    property::Property,
    surface_motion::SurfaceMotion,
};

/// The bounds every impl in this module shares, named once.
pub trait SurfaceAnimationDispatch:
    GlobalDispatch<CrownosSurfaceAnimationManagerV1, ()>
    + Dispatch<CrownosSurfaceAnimationManagerV1, ()>
    + Dispatch<CrownosAnimatedSurfaceV1, AnimatedSurfaceData>
    + SurfaceAnimationHandler
    + 'static
{
}

impl<D> SurfaceAnimationDispatch for D where
    D: GlobalDispatch<CrownosSurfaceAnimationManagerV1, ()>
        + Dispatch<CrownosSurfaceAnimationManagerV1, ()>
        + Dispatch<CrownosAnimatedSurfaceV1, AnimatedSurfaceData>
        + SurfaceAnimationHandler
        + 'static
{
}

impl<D: SurfaceAnimationDispatch> GlobalDispatch<CrownosSurfaceAnimationManagerV1, (), D>
    for SurfaceAnimationState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosSurfaceAnimationManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D: SurfaceAnimationDispatch> Dispatch<CrownosSurfaceAnimationManagerV1, (), D>
    for SurfaceAnimationState
{
    fn request(
        _state: &mut D,
        _client: &Client,
        manager: &CrownosSurfaceAnimationManagerV1,
        request: crownos_surface_animation_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_surface_animation_manager_v1::Request::GetAnimatedSurface { id, surface } =
            request
        else {
            return;
        };
        let object = data_init.init(id, AnimatedSurfaceData::new(&surface));

        if get_role(&surface) != Some(SUBSURFACE_ROLE) {
            manager.post_error(
                crownos_surface_animation_manager_v1::Error::NotSubsurface,
                "only a wl_subsurface can be animated",
            );
            return;
        }

        let (claimed, needs_hook) = with_states(&surface, |states| {
            SurfaceMotion::of(states).map_or((false, false), |motion| {
                let claimed = motion.claim(&object);
                (claimed, claimed && motion.take_hook_registration())
            })
        });
        if !claimed {
            manager.post_error(
                crownos_surface_animation_manager_v1::Error::AlreadyAnimated,
                "the surface already has an animated surface object",
            );
            return;
        }
        if needs_hook {
            add_post_commit_hook::<D, _>(&surface, |state, _dh, surface| {
                apply_committed(state, surface);
            });
        }
    }
}

impl<D: SurfaceAnimationDispatch> Dispatch<CrownosAnimatedSurfaceV1, AnimatedSurfaceData, D>
    for SurfaceAnimationState
{
    fn request(
        _state: &mut D,
        _client: &Client,
        object: &CrownosAnimatedSurfaceV1,
        request: crownos_animated_surface_v1::Request,
        data: &AnimatedSurfaceData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let Some(surface) = data.wl_surface() else {
            return;
        };
        match request {
            crownos_animated_surface_v1::Request::Animate {
                property,
                target,
                response_seconds,
                damping_ratio,
                start_time_hi,
                start_time_lo,
            } => {
                let Some(property) = valid_property(object, property) else {
                    return;
                };
                if response_seconds <= 0.0 || damping_ratio < 0.0 {
                    object.post_error(
                        crownos_animated_surface_v1::Error::InvalidSpring,
                        "a spring needs a positive response and a non-negative damping ratio",
                    );
                    return;
                }
                let change = PropertyChange::Animate {
                    target: target as f32,
                    spring: Spring::new(response_seconds as f32, damping_ratio as f32),
                    start: start_time(start_time_hi, start_time_lo),
                };
                with_pending(&surface, |pending| pending.change(property, change));
            }
            crownos_animated_surface_v1::Request::Set { property, value } => {
                let Some(property) = valid_property(object, property) else {
                    return;
                };
                with_pending(&surface, |pending| {
                    pending.change(property, PropertyChange::Set(value as f32));
                });
            }
            crownos_animated_surface_v1::Request::Destroy => {
                with_pending(&surface, PendingMotion::reset);
            }
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut D,
        _client: ClientId,
        object: &CrownosAnimatedSurfaceV1,
        data: &AnimatedSurfaceData,
    ) {
        let Some(surface) = data.wl_surface() else {
            return;
        };
        with_states(&surface, |states| {
            if let Some(motion) = SurfaceMotion::of(states) {
                motion.release(object);
            }
        });
    }
}

fn valid_property(
    object: &CrownosAnimatedSurfaceV1,
    property: WEnum<Property>,
) -> Option<Property> {
    let WEnum::Value(property) = property else {
        object.post_error(
            crownos_animated_surface_v1::Error::InvalidProperty,
            "unknown property",
        );
        return None;
    };
    Some(property)
}

fn start_time(high: u32, low: u32) -> Option<Duration> {
    let micros = (u64::from(high) << 32) | u64::from(low);
    (micros != 0).then(|| Duration::from_micros(micros))
}

fn with_pending(surface: &WlSurface, change: impl FnOnce(&mut PendingMotion)) {
    with_states(surface, |states| {
        change(states.cached_state.get::<PendingMotion>().pending());
    });
}

fn apply_committed<D: SurfaceAnimationHandler>(state: &mut D, surface: &WlSurface) {
    let now = Duration::from(Clock::<Monotonic>::new().now());
    let moving = with_states(surface, |states| {
        let committed = std::mem::take(states.cached_state.get::<PendingMotion>().current());
        if committed == PendingMotion::default() {
            return None;
        }
        let motion = SurfaceMotion::of(states)?;
        let owner = motion.owner();
        let moving = motion.update(|tracks| {
            tracks.apply(&committed, now, &mut |event| {
                if let Some(owner) = &owner {
                    send(owner, event);
                }
            });
            tracks.is_moving()
        });
        owner.filter(|_| moving)
    });
    if let Some(owner) = moving {
        state.surface_animations().track(&owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_times_join_both_halves_and_zero_means_now() {
        assert_eq!(start_time(0, 0), None);
        assert_eq!(start_time(0, 1_500), Some(Duration::from_micros(1_500)));
        assert_eq!(start_time(1, 0), Some(Duration::from_micros(1_u64 << 32)));
    }
}

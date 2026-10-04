//! The surfaces whose properties are moving, stepped once per output frame.

use std::time::Duration;

use crownos_protocols::surface_animation::v1::server::crownos_animated_surface_v1::CrownosAnimatedSurfaceV1;
use smithay::wayland::compositor::with_states;
use wayland_server::{Resource, Weak};

use super::{AnimatedSurfaceData, surface_motion::SurfaceMotion, tracks::MotionEvent};

#[derive(Debug, Default)]
pub struct SurfaceAnimations {
    animating: Vec<Weak<CrownosAnimatedSurfaceV1>>,
}

impl SurfaceAnimations {
    /// Asked every frame to decide whether to schedule another.
    pub fn is_animating(&self) -> bool {
        !self.animating.is_empty()
    }

    /// Samples every running spring at `now`, a CLOCK_MONOTONIC instant, and
    /// tells clients which ones came to rest.
    pub fn advance(&mut self, now: Duration) {
        self.animating.retain(|object| advance_object(object, now));
    }

    pub(super) fn track(&mut self, object: &CrownosAnimatedSurfaceV1) {
        let id = object.id();
        if !self.animating.iter().any(|weak| weak.id() == id) {
            self.animating.push(object.downgrade());
        }
    }
}

fn advance_object(object: &Weak<CrownosAnimatedSurfaceV1>, now: Duration) -> bool {
    let Ok(object) = object.upgrade() else {
        return false;
    };
    let Some(surface) = object
        .data::<AnimatedSurfaceData>()
        .and_then(AnimatedSurfaceData::wl_surface)
    else {
        return false;
    };
    with_states(&surface, |states| {
        SurfaceMotion::of(states).is_some_and(|motion| {
            motion.update(|tracks| {
                tracks.advance(now, &mut |event| send(&object, event));
                tracks.is_moving()
            })
        })
    })
}

pub(super) fn send(object: &CrownosAnimatedSurfaceV1, event: MotionEvent) {
    match event {
        MotionEvent::Settled { property, value } => object.settled(property, f64::from(value)),
        MotionEvent::Interrupted {
            property,
            value,
            velocity,
        } => object.interrupted(property, f64::from(value), f64::from(velocity)),
    }
}

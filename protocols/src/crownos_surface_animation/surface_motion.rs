//! A surface's animation state, kept in its `data_map` so the renderer finds
//! it while walking the surface tree.
//!
//! Inserted non-threadsafe: every access comes from the compositor thread, and
//! `Cell` lets the commit hook and the frame step update it in place without a
//! lock.

use std::cell::Cell;

use crownos_protocols::surface_animation::v1::server::crownos_animated_surface_v1::CrownosAnimatedSurfaceV1;
use smithay::wayland::compositor::SurfaceData;
use wayland_server::{Resource, Weak};

use super::{tracks::Tracks, transform::SurfaceTransform};

#[derive(Default)]
pub struct SurfaceMotion {
    tracks: Cell<Tracks>,
    transform: Cell<SurfaceTransform>,
    owner: Cell<Option<Weak<CrownosAnimatedSurfaceV1>>>,
    hooked: Cell<bool>,
}

impl SurfaceMotion {
    pub fn of(states: &SurfaceData) -> Option<&Self> {
        states.data_map.insert_if_missing(Self::default);
        states.data_map.get::<Self>()
    }

    pub fn update<T>(&self, change: impl FnOnce(&mut Tracks) -> T) -> T {
        let mut tracks = self.tracks.get();
        let result = change(&mut tracks);
        self.transform.set(tracks.transform());
        self.tracks.set(tracks);
        result
    }

    pub fn owner(&self) -> Option<CrownosAnimatedSurfaceV1> {
        let owner = self.owner.take();
        let live = owner.as_ref().and_then(|weak| weak.upgrade().ok());
        self.owner.set(owner);
        live
    }

    /// Takes the surface's one animation slot, unless a live object holds it.
    pub fn claim(&self, object: &CrownosAnimatedSurfaceV1) -> bool {
        if self.owner().is_some() {
            return false;
        }
        self.owner.set(Some(object.downgrade()));
        true
    }

    /// Frees the slot, but only for the object holding it: a rejected
    /// duplicate must not evict the rightful owner.
    pub fn release(&self, object: &CrownosAnimatedSurfaceV1) {
        let owner = self.owner.take();
        let held_by_object = owner.as_ref().is_some_and(|weak| weak.id() == object.id());
        if !held_by_object {
            self.owner.set(owner);
        }
    }

    /// True exactly once per surface, for registering the commit hook.
    pub fn take_hook_registration(&self) -> bool {
        !self.hooked.replace(true)
    }
}

/// The transform an animated subsurface is drawn with, or `None` when it is
/// drawn as committed.
pub fn surface_transform(states: &SurfaceData) -> Option<SurfaceTransform> {
    let transform = states.data_map.get::<SurfaceMotion>()?.transform.get();
    (!transform.is_identity()).then_some(transform)
}

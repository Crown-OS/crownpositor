use std::io;

use smithay::{
    reexports::{
        calloop::{EventSource, Interest},
        wayland_server::{Resource, protocol::wl_surface::WlSurface},
    },
    wayland::{
        compositor::{
            Blocker, BufferAssignment, CompositorHandler, SurfaceAttributes, add_blocker,
            add_pre_commit_hook, with_states,
        },
        dmabuf::get_dmabuf,
        drm_syncobj::{DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState},
    },
};

use crate::state::State;

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.wayland.drm_syncobj_state.as_mut()
    }
}

/// Holds a commit back until the GPU has finished writing its buffer, so the
/// compositor never samples, or scans out, a half-drawn frame and never
/// stalls its own GPU queue waiting for one.
///
/// Waits on the explicit-sync acquire point when the client sent one, and on
/// the dmabuf's implicit fence otherwise.
pub fn hold_commits_until_ready(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _, surface| {
        let (acquire_point, dmabuf) = with_states(surface, |states| {
            let acquire_point = states
                .cached_state
                .get::<DrmSyncobjCachedState>()
                .pending()
                .acquire_point
                .clone();
            let dmabuf = match states
                .cached_state
                .get::<SurfaceAttributes>()
                .pending()
                .buffer
                .as_ref()
            {
                Some(BufferAssignment::NewBuffer(buffer)) => get_dmabuf(buffer).ok().cloned(),
                _ => None,
            };
            (acquire_point, dmabuf)
        });
        let Some(dmabuf) = dmabuf else {
            return;
        };

        let held = acquire_point
            .and_then(|point| point.generate_blocker().ok())
            .is_some_and(|(blocker, source)| hold_until(state, surface, blocker, source));
        if !held && let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
            hold_until(state, surface, blocker, source);
        }
    });
}

/// Registers `blocker` on the surface and releases it when `source` fires.
fn hold_until<B, S>(state: &mut State, surface: &WlSurface, blocker: B, source: S) -> bool
where
    B: Blocker + Send + 'static,
    S: EventSource<Event = (), Ret = io::Result<()>> + 'static,
{
    let Some(client) = surface.client() else {
        return false;
    };
    let registered = state
        .common
        .event_loop_handle
        .insert_source(source, move |_, _, state| {
            let display_handle = state.common.display_handle.clone();
            state
                .client_compositor_state(&client)
                .blocker_cleared(state, &display_handle);
            Ok(())
        });
    if registered.is_err() {
        return false;
    }
    add_blocker(surface, blocker);
    true
}

//! Everything a render pass owes clients besides pixels: frame callbacks,
//! presentation feedback, FIFO and commit-timing barriers, and the record of
//! which output each surface is mainly shown on, which all of those key off.

use std::{collections::HashMap, time::Duration};

use smithay::{
    backend::renderer::element::{
        RenderElementStates, default_primary_scanout_output_compare, utils::select_dmabuf_feedback,
    },
    desktop::{
        layer_map_for_output,
        utils::{
            OutputPresentationFeedback, send_frames_surface_tree,
            surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
            take_presentation_feedback_surface_tree, update_surface_primary_scanout_output,
            with_surfaces_surface_tree,
        },
    },
    input::pointer::CursorImageStatus,
    output::Output,
    reexports::wayland_server::{
        Client, Resource, backend::ClientId, protocol::wl_surface::WlSurface,
    },
    utils::{Monotonic, Time},
    wayland::{
        commit_timing::CommitTimerBarrierStateUserData,
        compositor::{CompositorHandler, SurfaceData},
        dmabuf::DmabufFeedback,
        fifo::FifoBarrierCachedState,
    },
};

use crate::{
    rendering::cursor::Cursor,
    shell::{Shell, workspace::Workspace},
    state::State,
};

/// How often a surface nobody can see still hears a frame callback: enough to
/// keep it alive, not enough to cost anything.
const OCCLUDED_FRAME_THROTTLE: Duration = Duration::from_secs(1);

/// Clients whose held-back commits a barrier release lets through.
#[derive(Default)]
pub struct ReleasedClients(HashMap<ClientId, Client>);

impl ReleasedClients {
    fn insert(&mut self, surface: &WlSurface) {
        if let Some(client) = surface.client() {
            self.0.insert(client.id(), client);
        }
    }
}

impl State {
    /// Applies the commits the released barriers were holding back. Runs with
    /// no shell or backend borrow alive: it re-enters the commit handler.
    pub fn clear_blockers(&mut self, released: ReleasedClients) {
        let display_handle = self.common.display_handle.clone();
        for client in released.0.into_values() {
            self.client_compositor_state(&client)
                .blocker_cleared(self, &display_handle);
        }
    }
}

/// Every surface that can end up on `output`: all of its monitor's windows,
/// hidden workspaces included so they are known to be hidden, its layer
/// surfaces, its lock surface, and the cursor.
fn for_each_surface(
    shell: &Shell,
    output: &Output,
    cursor: &Cursor,
    mut processor: impl FnMut(&WlSurface, &SurfaceData),
) {
    if let Some(monitor) = shell.monitor(output) {
        for tile in monitor.workspaces().iter().flat_map(Workspace::tiles) {
            tile.window().with_surfaces(&mut processor);
        }
    }
    for layer in layer_map_for_output(output).layers() {
        layer.with_surfaces(&mut processor);
    }
    if let Some(lock) = shell.session_lock.surface_on(output) {
        with_surfaces_surface_tree(lock.wl_surface(), &mut processor);
    }
    if let CursorImageStatus::Surface(surface) = &cursor.status {
        with_surfaces_surface_tree(surface, &mut processor);
    }
}

/// Records which output each surface was mainly shown on this frame. A
/// surface this frame did not show loses this output.
pub fn track_primary_scanout(
    shell: &Shell,
    output: &Output,
    cursor: &Cursor,
    states: &RenderElementStates,
) {
    for_each_surface(shell, output, cursor, |surface, data| {
        update_surface_primary_scanout_output(
            surface,
            output,
            data,
            None,
            states,
            default_primary_scanout_output_compare,
        );
    });
}

/// Lets through every `wp_commit_timer_v1` commit due by the time this frame
/// is shown. Returns whether a later one is still waiting, which keeps the
/// output rendering until it is due.
pub fn signal_commit_timers(
    shell: &Shell,
    output: &Output,
    cursor: &Cursor,
    presentation_time: Duration,
    released: &mut ReleasedClients,
) -> bool {
    let deadline = Time::<Monotonic>::from(presentation_time);
    let mut waiting = false;
    for_each_surface(shell, output, cursor, |surface, data| {
        let Some(barriers) = data.data_map.get::<CommitTimerBarrierStateUserData>() else {
            return;
        };
        let Ok(mut barriers) = barriers.lock() else {
            return;
        };
        if barriers.signal_until(deadline) {
            released.insert(surface);
        }
        waiting |= barriers.next_deadline().is_some();
    });
    waiting
}

/// Releases the `wp_fifo_v1` barrier of everything this output shows, and of
/// everything no output shows, which must not stall forever.
pub fn signal_fifo_barriers(
    shell: &Shell,
    output: &Output,
    cursor: &Cursor,
    released: &mut ReleasedClients,
) {
    for_each_surface(shell, output, cursor, |surface, data| {
        if surface_primary_scanout_output(surface, data).is_some_and(|primary| primary != *output) {
            return;
        }
        let barrier = data
            .cached_state
            .get::<FifoBarrierCachedState>()
            .current()
            .barrier
            .take();
        if let Some(barrier) = barrier {
            barrier.signal();
            released.insert(surface);
        }
    });
}

/// What lets clients draw their *next* frame: at the output's rate for what it
/// shows, throttled for what nothing shows.
pub fn send_frame_callbacks(shell: &Shell, output: &Output, cursor: &Cursor, now: Duration) {
    let throttle = Some(OCCLUDED_FRAME_THROTTLE);
    if let Some(monitor) = shell.monitor(output) {
        for tile in monitor.workspaces().iter().flat_map(Workspace::tiles) {
            tile.window()
                .send_frame(output, now, throttle, surface_primary_scanout_output);
        }
    }
    for layer in layer_map_for_output(output).layers() {
        layer.send_frame(output, now, throttle, surface_primary_scanout_output);
    }
    if let Some(lock) = shell.session_lock.surface_on(output) {
        send_frames_surface_tree(
            lock.wl_surface(),
            output,
            now,
            throttle,
            surface_primary_scanout_output,
        );
    }
    // A client's cursor surface is in neither the shell model nor the layer
    // map, so it needs its own callback or an animated cursor draws once.
    cursor.send_frame(output, now, throttle);
}

/// Points each visible surface at the planes when its buffer could have been
/// scanned out but was not, and at the compositing GPU otherwise. Unchanged
/// feedback is not resent.
pub fn send_dmabuf_feedback(
    shell: &Shell,
    output: &Output,
    render: &DmabufFeedback,
    scanout: &DmabufFeedback,
    states: &RenderElementStates,
) {
    let select = |surface: &WlSurface, _: &SurfaceData| {
        select_dmabuf_feedback(surface, states, render, scanout)
    };
    if let Some(monitor) = shell.monitor(output) {
        for tile in shell.visible_windows(monitor) {
            tile.window()
                .send_dmabuf_feedback(output, surface_primary_scanout_output, select);
        }
    }
    for layer in layer_map_for_output(output).layers() {
        layer.send_dmabuf_feedback(output, surface_primary_scanout_output, select);
    }
}

/// Every presentation-feedback callback of what this frame shows, to resolve
/// when it reaches the screen. Zero-copy is flagged per surface.
pub fn take_presentation_feedbacks(
    shell: &Shell,
    output: &Output,
    states: &RenderElementStates,
) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);
    let flags = |surface: &WlSurface, _: &SurfaceData| {
        surface_presentation_feedback_flags_from_states(surface, None, states)
    };

    if let Some(monitor) = shell.monitor(output) {
        for tile in shell.visible_windows(monitor) {
            tile.window().take_presentation_feedback(
                &mut feedback,
                surface_primary_scanout_output,
                flags,
            );
        }
    }
    for layer in layer_map_for_output(output).layers() {
        layer.take_presentation_feedback(&mut feedback, surface_primary_scanout_output, flags);
    }
    if let Some(lock) = shell.session_lock.surface_on(output) {
        take_presentation_feedback_surface_tree(
            lock.wl_surface(),
            &mut feedback,
            surface_primary_scanout_output,
            flags,
        );
    }

    feedback
}

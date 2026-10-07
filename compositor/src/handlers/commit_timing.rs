use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::{
        commit_timing::CommitTimerStateUserData,
        compositor::{add_pre_commit_hook, with_states},
    },
};

use crate::state::State;

/// A timed commit is held back until a frame is due for it, and only a render
/// pass releases it. Held, it never reaches the commit handler that schedules
/// one, so an idle output would keep it waiting for unrelated damage — a
/// cursor move. Schedule the frame as the commit arrives instead.
pub fn render_timed_commits(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _, surface| {
        if has_pending_timestamp(surface) {
            state.queue_redraw_for_surface(surface);
        }
    });
}

fn has_pending_timestamp(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CommitTimerStateUserData>()
            .is_some_and(|timer| timer.borrow().timestamp.is_some())
    })
}

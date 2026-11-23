mod backend;
mod brightness;
mod color;
mod controls;
mod handlers;
mod input;
mod layout;
mod logging;
mod menu;
mod rendering;
mod shaders;
mod shell;
mod state;
mod utils;
mod xwayland;

use anyhow::Context;
use calloop::EventLoop;

use crate::state::State;
pub use crate::state::{Exit, RESTART_SESSION_STATUS};

pub fn run() -> anyhow::Result<Exit> {
    logging::init();

    let mut event_loop =
        EventLoop::<State>::try_new().with_context(|| "Failed to initialize the event loop")?;
    let mut state = State::try_new(&mut event_loop)?;

    let preference = backend::Preference::detect();
    match preference {
        backend::Preference::Winit => backend::winit::init(&mut state)?,
        backend::Preference::Kms => backend::kms::init(&mut state)?,
    }
    tracing::info!(backend = state.backend.name(), "backend started");
    state.focus_primary();
    state.configure_capture_sync();
    state.init_lock_screen();

    // Safety: no other thread is reading the environment yet.
    unsafe { state::session_env::enter_wayland_session(&state.common.socket_name) };
    tracing::info!(socket = ?state.common.socket_name, "crownpositor is running");

    // A nested session must not repoint the host's activated services at it.
    let owns_session_bus = preference == backend::Preference::Kms;
    if owns_session_bus {
        state.export_session_environment();
    }

    // Outputs exist and the socket is live, so a bar or wallpaper that connects
    // immediately has something to anchor to.
    state.start_session_apps();
    state.start_control_service();

    event_loop
        .run(None, &mut state, |state| {
            state.shell.refresh();
            state.refresh_pointer_bounds();
            // Both the renderer and the hit test read the menu geometry, so it
            // is settled before either of them runs.
            state.layout_menus();
            // A layer surface that maps, changes its keyboard interactivity or
            // goes away moves focus with no input event to hang the decision
            // off, so the reconcile belongs here rather than at every mutation.
            state.update_keyboard_focus();
            state.update_pointer_focus();
            state.refresh_idle_inhibit();
            state.confirm_session_lock();
            state.refresh_lock_screen();
            state.shell.popups.cleanup();
            state.refresh_foreign_toplevels();
            state.refresh_ext_workspaces();
            state.refresh_workspace_mode();
            state.refresh_output_info();
            state.apply_software_dim();
            state.refresh_preferred_descriptions();
            state.sync_window_directory();
            state.refresh_virtual_output_titles();
            // Frames queued during dispatch render here, after the burst of
            // events that requested them has been fully drained.
            backend::kms::redraw_queued_outputs(state);
            backend::render_offscreen(state);
            // After rendering, so springs that landed this frame are seen.
            state.refresh_surface_visibility();
            state.refresh_app_icons();
            let _ = state.common.display_handle.flush_clients();
        })
        .with_context(|| "The event loop stopped unexpectedly")?;
    Ok(state.common.exit)
}

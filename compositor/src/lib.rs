mod backend;
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

pub fn run() -> anyhow::Result<()> {
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
    state.configure_capture_sync();

    // Point child processes at our socket rather than the host compositor.
    // Safety: no other thread is reading the environment yet.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.common.socket_name);
        if std::env::var_os("XDG_CURRENT_DESKTOP").is_none() {
            std::env::set_var("XDG_CURRENT_DESKTOP", state::session_env::DESKTOP_NAME);
        }
    }
    tracing::info!(socket = ?state.common.socket_name, "crownpositor is running");

    // A nested session must not repoint the host's activated services at it.
    if preference == backend::Preference::Kms {
        state.export_session_environment();
    }

    // Outputs exist and the socket is live, so a bar or wallpaper that connects
    // immediately has something to anchor to.
    state.run_startup();

    event_loop
        .run(None, &mut state, |state| {
            state.shell.refresh();
            // A layer surface that maps, changes its keyboard interactivity or
            // goes away moves focus with no input event to hang the decision
            // off, so the reconcile belongs here rather than at every mutation.
            state.update_keyboard_focus();
            state.update_pointer_focus();
            state.refresh_idle_inhibit();
            state.confirm_session_lock();
            state.shell.popups.cleanup();
            // Frames queued during dispatch render here, after the burst of
            // events that requested them has been fully drained.
            backend::kms::redraw_queued_outputs(state);
            backend::render_offscreen(state);
            // After rendering, so springs that landed this frame are seen.
            state.refresh_surface_visibility();
            let _ = state.common.display_handle.flush_clients();
        })
        .with_context(|| "The event loop stopped unexpectedly")
}

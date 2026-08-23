mod animations;
mod backend;
mod handlers;
mod input;
mod layout;
mod logging;
mod protocols;
mod rendering;
mod shaders;
mod shell;
mod state;
mod utils;

use anyhow::Context;
use calloop::EventLoop;

use crate::state::State;

pub fn run() -> anyhow::Result<()> {
    logging::init();

    let mut event_loop =
        EventLoop::<State>::try_new().with_context(|| "Failed to initialize the event loop")?;
    let mut state = State::try_new(&mut event_loop)?;

    match backend::Preference::detect() {
        backend::Preference::Winit => backend::winit::init(&mut state)?,
        backend::Preference::Kms => backend::kms::init(&mut state)?,
    }
    tracing::info!(backend = state.backend.name(), "backend started");

    // Point child processes at our socket rather than the host compositor.
    //
    // Safety: no other thread is reading the environment yet.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.common.socket_name) };
    tracing::info!(socket = ?state.common.socket_name, "crownpositor is running");

    // Outputs exist and the socket is live, so a bar or wallpaper that connects
    // immediately has something to anchor to.
    state.run_startup();

    event_loop
        .run(None, &mut state, |state| {
            state.shell.refresh();
            state.shell.popups.cleanup();
            // Frames queued during dispatch render here, after the burst of
            // events that requested them has been fully drained.
            backend::kms::redraw_queued_outputs(state);
            let _ = state.common.display_handle.flush_clients();
        })
        .with_context(|| "The event loop stopped unexpectedly")
}

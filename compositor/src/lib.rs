mod animations;
mod backend;
mod handlers;
mod input;
mod layout;
mod logging;
mod shell;
mod state;
mod theme;
mod utils;

use anyhow::Context;
use calloop::EventLoop;

use crate::state::State;

pub fn run() -> anyhow::Result<()> {
    logging::init();

    let mut event_loop =
        EventLoop::<State>::try_new().with_context(|| "Failed to initialize the event loop")?;
    let mut state = State::try_new(&mut event_loop)?;

    backend::winit::init(&mut state)?;

    // Point child processes at our socket rather than the host compositor.
    //
    // Safety: no other thread is reading the environment yet.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.common.socket_name) };
    tracing::info!(socket = ?state.common.socket_name, "crownpositor is running");

    event_loop
        .run(None, &mut state, |state| {
            state.shell.space.refresh();
            state.shell.popups.cleanup();
            let _ = state.common.display_handle.flush_clients();
        })
        .with_context(|| "The event loop stopped unexpectedly")
}

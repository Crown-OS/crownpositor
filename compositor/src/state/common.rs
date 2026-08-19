use std::{ffi::OsString, sync::Arc, time::Instant};

use anyhow::Context;
use calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, generic::Generic};
use parking_lot::Once;
use smithay::{
    reexports::wayland_server::{Display, DisplayHandle},
    wayland::socket::ListeningSocketSource,
};

use crate::{
    state::{State, client::ClientState},
    utils::runtime::TaskSender,
};

pub struct CommonState {
    pub event_loop_handle: LoopHandle<'static, State>,
    pub event_loop_signal: LoopSignal,
    pub display_handle: DisplayHandle,
    pub socket_name: OsString,
    pub start_time: Instant,
    /// Spawns async work on the Tokio pool; completions come back through the
    /// event loop, so the rendering thread never blocks on them.
    pub tasks: TaskSender,

    pub ready: Once,
}

impl CommonState {
    pub fn try_new(event_loop: &mut EventLoop<'static, State>) -> anyhow::Result<Self> {
        let display =
            Display::<State>::new().with_context(|| "Failed to initialize wayland display")?;
        let display_handle = display.handle();

        let socket_name = Self::init_wayland_display(display, event_loop)?;
        let tasks = TaskSender::init(&event_loop.handle())?;

        Ok(Self {
            event_loop_signal: event_loop.get_signal(),
            event_loop_handle: event_loop.handle(),
            display_handle,
            socket_name,
            start_time: Instant::now(),
            tasks,
            ready: Once::new(),
        })
    }

    fn init_wayland_display(
        display: Display<State>,
        event_loop: &mut EventLoop<'static, State>,
    ) -> anyhow::Result<OsString> {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket =
            ListeningSocketSource::new_auto().with_context(|| "Failed to bind a wayland socket")?;

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                state
                    .common
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        Ok(socket_name)
    }
}

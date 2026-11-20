use std::{os::unix::net::UnixStream, sync::Arc};

use crownos_protocols::output_brightness::v1::client::{
    crownos_output_brightness_manager_v1::CrownosOutputBrightnessManagerV1 as ClientManager,
    crownos_output_brightness_v1::{
        self as client_brightness, CrownosOutputBrightnessV1 as ClientBrightness,
    },
};
use wayland_client::{
    Connection, Dispatch as ClientDispatch, EventQueue, QueueHandle,
    protocol::{
        wl_output::{self as client_output, WlOutput as ClientOutput},
        wl_registry::{self, WlRegistry},
    },
};
use wayland_server::{
    DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, delegate_dispatch,
    delegate_global_dispatch, protocol::wl_output::WlOutput,
};

use super::*;

/// A server whose only `wl_output` is a bare global with no smithay output
/// behind it, so every brightness object it hands out is inert.
struct Server {
    state: OutputBrightnessState,
}

impl OutputBrightnessHandler for Server {
    fn output_brightness_state(&mut self) -> &mut OutputBrightnessState {
        &mut self.state
    }

    fn output_brightness(&mut self, _output: &Output) -> Option<OutputBrightness> {
        None
    }

    fn set_output_brightness(&mut self, _output: &Output, _level: u32) -> Option<OutputBrightness> {
        None
    }

    fn refresh_output_brightness(&mut self, _output: &Output) {}
}

impl GlobalDispatch<WlOutput, ()> for Server {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WlOutput>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<WlOutput, ()> for Server {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _output: &WlOutput,
        _request: <WlOutput as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

delegate_global_dispatch!(Server: [CrownosOutputBrightnessManagerV1: OutputBrightnessGlobalData] => OutputBrightnessState);
delegate_dispatch!(Server: [CrownosOutputBrightnessManagerV1: ()] => OutputBrightnessState);
delegate_dispatch!(Server: [CrownosOutputBrightnessV1: ()] => OutputBrightnessState);

#[derive(Default)]
struct ClientState {
    globals: Vec<(String, u32, u32)>,
    events: Vec<client_brightness::Event>,
}

impl ClientDispatch<WlRegistry, ()> for ClientState {
    fn event(
        client: &mut Self,
        _: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            client.globals.push((interface, version, name));
        }
    }
}

impl ClientDispatch<ClientOutput, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ClientOutput,
        _: client_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl ClientDispatch<ClientManager, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ClientManager,
        _: <ClientManager as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl ClientDispatch<ClientBrightness, ()> for ClientState {
    fn event(
        client: &mut Self,
        _: &ClientBrightness,
        event: client_brightness::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        client.events.push(event);
    }
}

struct Session {
    display: Display<Server>,
    server: Server,
    connection: Connection,
    queue: EventQueue<ClientState>,
    client: ClientState,
}

impl Session {
    fn start(privileged: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let display = Display::<Server>::new()?;
        let mut handle = display.handle();
        let server = Server {
            state: OutputBrightnessState::new::<Server, _>(&handle, move |_| privileged),
        };
        handle.create_global::<Server, WlOutput, ()>(4, ());
        let (server_end, client_end) = UnixStream::pair()?;
        handle.insert_client(server_end, Arc::new(()))?;
        let connection = Connection::from_socket(client_end)?;
        let queue = connection.new_event_queue();
        connection.display().get_registry(&queue.handle(), ());
        let mut session = Self {
            display,
            server,
            connection,
            queue,
            client: ClientState::default(),
        };
        session.exchange()?;
        Ok(session)
    }

    /// Lets requests and events cross a few times. A protocol error ends the
    /// exchange early; the caller reads it from the connection.
    fn exchange(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..3 {
            if self.queue.flush().is_err() {
                return Ok(());
            }
            self.display.dispatch_clients(&mut self.server)?;
            self.display.flush_clients()?;
            if let Some(guard) = self.queue.prepare_read() {
                let _ = guard.read();
            }
            if self.queue.dispatch_pending(&mut self.client).is_err() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn global(&self, interface: &str) -> Option<(u32, u32)> {
        self.client
            .globals
            .iter()
            .find(|(offered, _, _)| offered == interface)
            .map(|(_, version, name)| (*version, *name))
    }

    fn bound_brightness(&mut self) -> Result<ClientBrightness, Box<dyn std::error::Error>> {
        let registry = self
            .connection
            .display()
            .get_registry(&self.queue.handle(), ());
        let (_, manager_name) = self
            .global("crownos_output_brightness_manager_v1")
            .ok_or("no brightness manager")?;
        let (_, output_name) = self.global("wl_output").ok_or("no wl_output")?;
        let manager: ClientManager = registry.bind(manager_name, 1, &self.queue.handle(), ());
        let output: ClientOutput = registry.bind(output_name, 4, &self.queue.handle(), ());
        let brightness = manager.get_output(&output, &self.queue.handle(), ());
        self.exchange()?;
        Ok(brightness)
    }
}

#[test]
fn a_privileged_client_is_offered_the_manager_at_version_1()
-> Result<(), Box<dyn std::error::Error>> {
    let session = Session::start(true)?;
    assert_eq!(
        session
            .global("crownos_output_brightness_manager_v1")
            .map(|(version, _)| version),
        Some(1)
    );
    Ok(())
}

#[test]
fn an_unprivileged_client_is_not_offered_the_manager() -> Result<(), Box<dyn std::error::Error>> {
    let session = Session::start(false)?;
    assert_eq!(session.global("crownos_output_brightness_manager_v1"), None);
    Ok(())
}

#[test]
fn a_level_above_the_top_is_a_protocol_error() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::start(true)?;
    let brightness = session.bound_brightness()?;
    brightness.set_level(MAX_LEVEL + 1);
    session.exchange()?;

    let error = session
        .connection
        .protocol_error()
        .ok_or("the server accepted a level above 10000")?;
    assert_eq!(error.object_interface, "crownos_output_brightness_v1");
    assert_eq!(error.code, client_brightness::Error::InvalidLevel as u32);
    Ok(())
}

#[test]
fn an_object_for_an_output_the_compositor_does_not_have_stays_silent()
-> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::start(true)?;
    let brightness = session.bound_brightness()?;
    brightness.set_level(MAX_LEVEL);
    brightness.refresh();
    session.exchange()?;

    assert!(session.client.events.is_empty());
    assert!(session.connection.protocol_error().is_none());
    Ok(())
}

#[test]
fn levels_up_to_the_top_are_valid() {
    assert_eq!(checked_level(0), Ok(0));
    assert_eq!(checked_level(MAX_LEVEL), Ok(MAX_LEVEL));
    assert_eq!(
        checked_level(MAX_LEVEL + 1),
        Err(InvalidLevel(MAX_LEVEL + 1))
    );
}

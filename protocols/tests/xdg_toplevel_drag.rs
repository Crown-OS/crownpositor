//! Drives `xdg-toplevel-drag-v1` from a real client over a socket pair, for the
//! protocol errors: getting them wrong disconnects the client.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use protocols::xdg_toplevel_drag::{
    Attachment, ToplevelDrag, ToplevelDragData, XdgToplevelDragHandler, XdgToplevelDragState,
};
use wayland_client::{
    Connection, DispatchError, EventQueue, Proxy, QueueHandle,
    backend::WaylandError,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_compositor::WlCompositor, wl_data_device::WlDataDevice,
        wl_data_device_manager::WlDataDeviceManager, wl_data_source::WlDataSource,
        wl_registry::WlRegistry, wl_seat::WlSeat, wl_surface::WlSurface,
    },
};
use wayland_protocols::xdg::toplevel_drag::v1::{
    client::{
        xdg_toplevel_drag_manager_v1::XdgToplevelDragManagerV1,
        xdg_toplevel_drag_v1::XdgToplevelDragV1,
    },
    server::{
        xdg_toplevel_drag_manager_v1::XdgToplevelDragManagerV1 as ServerManager,
        xdg_toplevel_drag_v1::XdgToplevelDragV1 as ServerDrag,
    },
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New,
    backend::ClientData,
    delegate_dispatch, delegate_global_dispatch,
    protocol::{
        wl_compositor::{self as server_compositor, WlCompositor as ServerCompositor},
        wl_data_device::{self as server_device, WlDataDevice as ServerDevice},
        wl_data_device_manager::{
            self as server_manager, WlDataDeviceManager as ServerDeviceManager,
        },
        wl_data_source::WlDataSource as ServerSource,
        wl_seat::WlSeat as ServerSeat,
        wl_surface::WlSurface as ServerSurface,
    },
};

/// Just enough of a compositor for a drag to start: `start_drag` marks the
/// source's toplevel drag as running, as the compositor's `dnd_requested` does.
struct Server {
    drags: XdgToplevelDragState,
}

impl XdgToplevelDragHandler for Server {
    fn xdg_toplevel_drag_state(&mut self) -> &mut XdgToplevelDragState {
        &mut self.drags
    }

    fn toplevel_attached(&mut self, _drag: &ToplevelDrag, _attachment: &Attachment) {}
}

delegate_global_dispatch!(Server: [ServerManager: ()] => XdgToplevelDragState);
delegate_dispatch!(Server: [ServerManager: ()] => XdgToplevelDragState);
delegate_dispatch!(Server: [ServerDrag: ToplevelDragData] => XdgToplevelDragState);

impl GlobalDispatch<ServerDeviceManager, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ServerDeviceManager>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl GlobalDispatch<ServerSeat, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ServerSeat>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ServerDeviceManager, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ServerDeviceManager,
        request: server_manager::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            server_manager::Request::CreateDataSource { id } => {
                data_init.init(id, ());
            }
            server_manager::Request::GetDataDevice { id, .. } => {
                data_init.init(id, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<ServerDevice, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ServerDevice,
        request: server_device::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let server_device::Request::StartDrag {
            source: Some(source),
            ..
        } = request
            && let Some(drag) = state.drags.drag_for(&source)
        {
            drag.start();
        }
    }
}

impl Dispatch<ServerSource, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ServerSource,
        _: wayland_server::protocol::wl_data_source::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl GlobalDispatch<ServerCompositor, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ServerCompositor>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ServerCompositor, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ServerCompositor,
        request: server_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let server_compositor::Request::CreateSurface { id } = request {
            data_init.init(id, ());
        }
    }
}

impl Dispatch<ServerSurface, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ServerSurface,
        _: wayland_server::protocol::wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ServerSeat, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ServerSeat,
        _: wayland_server::protocol::wl_seat::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

struct TestClient;
impl ClientData for TestClient {}

struct ClientState;

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for ClientState {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(ClientState: ignore WlDataDeviceManager);
wayland_client::delegate_noop!(ClientState: ignore WlDataDevice);
wayland_client::delegate_noop!(ClientState: ignore WlDataSource);
wayland_client::delegate_noop!(ClientState: ignore WlSeat);
wayland_client::delegate_noop!(ClientState: ignore WlCompositor);
wayland_client::delegate_noop!(ClientState: ignore WlSurface);
wayland_client::delegate_noop!(ClientState: ignore XdgToplevelDragManagerV1);
wayland_client::delegate_noop!(ClientState: ignore XdgToplevelDragV1);

/// A compositor on its own thread, stopped when dropped.
struct Harness {
    running: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
    queue: EventQueue<ClientState>,
    manager: XdgToplevelDragManagerV1,
    data_devices: WlDataDeviceManager,
    seat: WlSeat,
    compositor: WlCompositor,
}

impl Harness {
    fn new() -> Self {
        let (server_socket, client_socket) = UnixStream::pair().expect("socket pair");
        let running = Arc::new(AtomicBool::new(true));
        let server = {
            let running = running.clone();
            thread::spawn(move || serve(server_socket, &running))
        };

        let connection = Connection::from_socket(client_socket).expect("client connection");
        let (globals, queue) = registry_queue_init::<ClientState>(&connection).expect("registry");
        let handle = queue.handle();
        Self {
            manager: globals.bind(&handle, 1..=1, ()).expect("toplevel drag"),
            data_devices: globals.bind(&handle, 1..=3, ()).expect("data devices"),
            seat: globals.bind(&handle, 1..=1, ()).expect("seat"),
            compositor: globals.bind(&handle, 1..=1, ()).expect("compositor"),
            running,
            server: Some(server),
            queue,
        }
    }

    fn source(&self) -> WlDataSource {
        self.data_devices
            .create_data_source(&self.queue.handle(), ())
    }

    fn drag(&self, source: &WlDataSource) -> XdgToplevelDragV1 {
        self.manager
            .get_xdg_toplevel_drag(source, &self.queue.handle(), ())
    }

    fn start_drag(&self, source: &WlDataSource) {
        let handle = self.queue.handle();
        let device = self.data_devices.get_data_device(&self.seat, &handle, ());
        let origin = self.compositor.create_surface(&handle, ());
        device.start_drag(Some(source), &origin, None, 0);
    }

    /// The protocol error the client was disconnected with, if any, as the
    /// object's protocol id and the error code. By id: a client that already
    /// destroyed the object can no longer name its interface.
    fn error(&mut self) -> Option<(u32, u32)> {
        match self.queue.roundtrip(&mut ClientState) {
            Ok(_) => None,
            Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
                Some((error.object_id, error.code))
            }
            Err(other) => panic!("unexpected failure: {other}"),
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn serve(socket: UnixStream, running: &AtomicBool) {
    let mut display = Display::<Server>::new().expect("display");
    let mut handle = display.handle();
    let mut state = Server {
        drags: XdgToplevelDragState::new::<Server>(&handle),
    };
    handle.create_global::<Server, ServerDeviceManager, _>(3, ());
    handle.create_global::<Server, ServerSeat, _>(1, ());
    handle.create_global::<Server, ServerCompositor, _>(1, ());
    handle
        .insert_client(socket, Arc::new(TestClient))
        .expect("insert client");
    while running.load(Ordering::Acquire) {
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();
        thread::sleep(Duration::from_millis(1));
    }
}

const INVALID_SOURCE: u32 = 0;
const ONGOING_DRAG: u32 = 1;

#[test]
fn a_source_carries_one_drag_at_most() {
    let mut harness = Harness::new();
    let source = harness.source();
    harness.drag(&source);
    harness.drag(&source);
    let manager = harness.manager.id().protocol_id();
    assert_eq!(harness.error(), Some((manager, INVALID_SOURCE)));
}

#[test]
fn a_drag_that_never_started_can_go() {
    let mut harness = Harness::new();
    let source = harness.source();
    harness.drag(&source).destroy();
    assert_eq!(harness.error(), None);
}

#[test]
fn a_running_drag_cannot_go() {
    let mut harness = Harness::new();
    let source = harness.source();
    let drag = harness.drag(&source);
    harness.start_drag(&source);
    drag.destroy();
    // The client forgot the object as it sent the destroy, so wayland-client
    // reports the error against id 0: the code is all there is to check.
    assert_eq!(harness.error().map(|(_, code)| code), Some(ONGOING_DRAG));
}

#[test]
fn destroying_the_source_ends_the_drag_for_its_client() {
    // Chromium cancels a tab drag by destroying the source, then the drag.
    let mut harness = Harness::new();
    let source = harness.source();
    let drag = harness.drag(&source);
    harness.start_drag(&source);
    source.destroy();
    drag.destroy();
    assert_eq!(harness.error(), None);
}

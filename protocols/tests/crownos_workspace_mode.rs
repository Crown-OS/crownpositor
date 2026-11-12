//! Drives `crownos_workspace_mode_v1` from a real client over a socket pair.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crownos_protocols::workspace_mode::v1::{
    client::crownos_workspace_mode_manager_v1::{
        self as client_manager, CrownosWorkspaceModeManagerV1,
    },
    server::crownos_workspace_mode_manager_v1::CrownosWorkspaceModeManagerV1 as ServerManager,
};
use protocols::crownos_workspace_mode::{
    Mode, WorkspaceModeGlobalData, WorkspaceModeHandler, WorkspaceModeState,
};
use wayland_client::{
    Connection, DispatchError, EventQueue, Proxy, QueueHandle, WEnum,
    backend::WaylandError,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::WlRegistry,
};
use wayland_server::{Display, backend::ClientData, delegate_dispatch, delegate_global_dispatch};

const INVALID_MODE: u32 = 0;

/// A compositor with one workspace, which switches the moment it is asked.
struct Server {
    workspace_mode: WorkspaceModeState,
}

impl WorkspaceModeHandler for Server {
    fn workspace_mode_state(&mut self) -> &mut WorkspaceModeState {
        &mut self.workspace_mode
    }

    fn set_workspace_mode(&mut self, mode: Mode) {
        self.workspace_mode.update(mode);
    }
}

delegate_global_dispatch!(Server: [ServerManager: WorkspaceModeGlobalData] => WorkspaceModeState);
delegate_dispatch!(Server: [ServerManager: ()] => WorkspaceModeState);

struct TestClient;
impl ClientData for TestClient {}

#[derive(Default)]
struct ClientState {
    heard: Vec<client_manager::Mode>,
}

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

impl wayland_client::Dispatch<CrownosWorkspaceModeManagerV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &CrownosWorkspaceModeManagerV1,
        event: client_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let client_manager::Event::Mode {
            mode: WEnum::Value(mode),
        } = event
        {
            state.heard.push(mode);
        }
    }
}

/// A compositor on its own thread, stopped when dropped.
struct Harness {
    running: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
    queue: EventQueue<ClientState>,
    client: ClientState,
    manager: CrownosWorkspaceModeManagerV1,
}

impl Harness {
    fn new(initial: Option<Mode>) -> Self {
        let (server_socket, client_socket) = UnixStream::pair().expect("socket pair");
        let running = Arc::new(AtomicBool::new(true));
        let server = {
            let running = running.clone();
            thread::spawn(move || serve(server_socket, initial, &running))
        };

        let connection = Connection::from_socket(client_socket).expect("client connection");
        let (globals, queue) = registry_queue_init::<ClientState>(&connection).expect("registry");
        let manager = globals
            .bind(&queue.handle(), 1..=1, ())
            .expect("workspace mode");
        Self {
            running,
            server: Some(server),
            queue,
            client: ClientState::default(),
            manager,
        }
    }

    fn roundtrip(&mut self) -> Result<(), DispatchError> {
        self.queue.roundtrip(&mut self.client).map(drop)
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

fn serve(socket: UnixStream, initial: Option<Mode>, running: &AtomicBool) {
    let mut display = Display::<Server>::new().expect("display");
    let mut handle = display.handle();
    let mut state = Server {
        workspace_mode: WorkspaceModeState::new::<Server, _>(&handle, |_| true),
    };
    if let Some(mode) = initial {
        state.workspace_mode.update(mode);
    }
    handle
        .insert_client(socket, Arc::new(TestClient))
        .expect("insert client");
    while running.load(Ordering::Acquire) {
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_manager_hears_the_mode_as_it_binds() {
    let mut harness = Harness::new(Some(Mode::Floating));
    harness.roundtrip().expect("roundtrip");
    assert_eq!(harness.client.heard, [client_manager::Mode::Floating]);
}

#[test]
fn nothing_is_said_before_the_compositor_knows_the_mode() {
    let mut harness = Harness::new(None);
    harness.roundtrip().expect("roundtrip");
    assert!(harness.client.heard.is_empty());
}

#[test]
fn switching_is_answered_with_the_new_mode() {
    let mut harness = Harness::new(Some(Mode::Tiling));
    harness.manager.set_mode(client_manager::Mode::Floating);
    harness.roundtrip().expect("roundtrip");
    assert_eq!(
        harness.client.heard,
        [client_manager::Mode::Tiling, client_manager::Mode::Floating]
    );
}

#[test]
fn asking_for_the_current_mode_says_nothing_more() {
    let mut harness = Harness::new(Some(Mode::Tiling));
    harness.manager.set_mode(client_manager::Mode::Tiling);
    harness.roundtrip().expect("roundtrip");
    assert_eq!(harness.client.heard, [client_manager::Mode::Tiling]);
}

#[test]
fn a_mode_that_does_not_exist_is_a_protocol_error() {
    let mut harness = Harness::new(Some(Mode::Tiling));
    let manager = harness.manager.id().protocol_id();
    harness
        .manager
        .send_request(client_manager::Request::SetMode {
            mode: WEnum::Unknown(7),
        })
        .expect("send");
    match harness.roundtrip() {
        Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
            assert_eq!((error.object_id, error.code), (manager, INVALID_MODE));
        }
        other => panic!("expected invalid_mode, got {other:?}"),
    }
}

//! Drives `crownos_secret_input_v1` from a real client over a socket pair.

use std::{
    io::Write as _,
    os::{
        fd::{AsFd as _, OwnedFd},
        unix::net::UnixStream,
    },
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crownos_protocols::secret_input::v1::{
    client::{
        crownos_secret_input_manager_v1::CrownosSecretInputManagerV1,
        crownos_secret_input_request_v1::{
            self as client_request, CrownosSecretInputRequestV1, Kind,
        },
    },
    server::{
        crownos_secret_input_manager_v1::CrownosSecretInputManagerV1 as ServerManager,
        crownos_secret_input_request_v1::CrownosSecretInputRequestV1 as ServerRequest,
    },
};
use protocols::crownos_secret_input::{
    Outcome, RequestData, SecretInputGlobalData, SecretInputHandler, SecretInputRequest,
    SecretInputState, ValueKind,
};
use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
use wayland_client::{
    Connection, DispatchError, EventQueue, Proxy, QueueHandle, WEnum,
    backend::WaylandError,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::WlRegistry,
};
use wayland_server::{Display, backend::ClientData, delegate_dispatch, delegate_global_dispatch};

const ALREADY_COMMITTED: u32 = 0;
const NO_VALUE: u32 = 1;

/// A value as the compositor side read it: text, kind, Tab after.
type SeenValue = (String, ValueKind, bool);

/// What the compositor side saw.
#[derive(Default)]
struct Seen {
    committed: Vec<(String, Vec<SeenValue>)>,
    withdrawn: usize,
}

/// Refuses a request with a bad value at once and keeps the others open.
struct Server {
    seen: Arc<Mutex<Seen>>,
    open: Vec<SecretInputRequest>,
}

impl SecretInputHandler for Server {
    fn secret_input_committed(&mut self, request: SecretInputRequest) {
        if request.has_bad_value() {
            return request.finish(Outcome::Refused);
        }
        let values = request
            .values()
            .iter()
            .map(|value| (value.text.to_string(), value.kind, value.tab_after))
            .collect();
        if let Ok(mut seen) = self.seen.lock() {
            seen.committed
                .push((request.details().window_identifier.clone(), values));
        }
        self.open.push(request);
    }

    fn secret_input_withdrawn(&mut self, resource: &ServerRequest) {
        self.open.retain(|request| !request.is_for(resource));
        if let Ok(mut seen) = self.seen.lock() {
            seen.withdrawn += 1;
        }
    }
}

delegate_global_dispatch!(Server: [ServerManager: SecretInputGlobalData] => SecretInputState);
delegate_dispatch!(Server: [ServerManager: ()] => SecretInputState);
delegate_dispatch!(Server: [ServerRequest: RequestData] => SecretInputState);

struct TestClient;
impl ClientData for TestClient {}

#[derive(Default)]
struct ClientState {
    outcomes: Vec<client_request::Outcome>,
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

impl wayland_client::Dispatch<CrownosSecretInputManagerV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &CrownosSecretInputManagerV1,
        _: <CrownosSecretInputManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<CrownosSecretInputRequestV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &CrownosSecretInputRequestV1,
        event: client_request::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client_request::Event::Done {
            outcome: WEnum::Value(outcome),
        } = event
        else {
            return;
        };
        state.outcomes.push(outcome);
    }
}

/// A compositor on its own thread, stopped when dropped.
struct Harness {
    running: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
    seen: Arc<Mutex<Seen>>,
    queue: EventQueue<ClientState>,
    client: ClientState,
    manager: Option<CrownosSecretInputManagerV1>,
}

impl Harness {
    fn new(visible: bool) -> Self {
        let (server_socket, client_socket) = UnixStream::pair().expect("socket pair");
        let running = Arc::new(AtomicBool::new(true));
        let seen = Arc::new(Mutex::new(Seen::default()));
        let server = {
            let running = running.clone();
            let seen = seen.clone();
            thread::spawn(move || serve(server_socket, visible, seen, &running))
        };
        let connection = Connection::from_socket(client_socket).expect("client connection");
        let (globals, queue) = registry_queue_init::<ClientState>(&connection).expect("registry");
        let manager = globals.bind(&queue.handle(), 1..=1, ()).ok();
        Self {
            running,
            server: Some(server),
            seen,
            queue,
            client: ClientState::default(),
            manager,
        }
    }

    fn request(&self) -> CrownosSecretInputRequestV1 {
        self.manager.as_ref().expect("manager").create_request(
            "window-1".into(),
            "org.example.browser".into(),
            1,
            "agent".into(),
            "pass".into(),
            String::new(),
            &self.queue.handle(),
            (),
        )
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

fn serve(socket: UnixStream, visible: bool, seen: Arc<Mutex<Seen>>, running: &AtomicBool) {
    let mut display = Display::<Server>::new().expect("display");
    let mut handle = display.handle();
    SecretInputState::new::<Server, _>(&handle, move |_| visible);
    let mut state = Server {
        seen,
        open: Vec::new(),
    };
    handle
        .insert_client(socket, Arc::new(TestClient))
        .expect("insert client");
    while running.load(Ordering::Acquire) {
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();
        thread::sleep(Duration::from_millis(1));
    }
}

fn value(text: &[u8], seals: SealFlags) -> OwnedFd {
    let fd = memfd_create("value", MemfdFlags::ALLOW_SEALING | MemfdFlags::CLOEXEC).expect("memfd");
    let mut file = std::fs::File::from(fd);
    file.write_all(text).expect("write");
    let fd = OwnedFd::from(file);
    fcntl_add_seals(&fd, seals).expect("seal");
    fd
}

fn sealed(text: &[u8]) -> OwnedFd {
    value(
        text,
        SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
    )
}

fn expect_error(harness: &mut Harness, object: u32, code: u32) {
    match harness.roundtrip() {
        Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
            assert_eq!((error.object_id, error.code), (object, code));
        }
        other => panic!("expected protocol error {code}, got {other:?}"),
    }
}

#[test]
fn only_the_filtered_client_sees_the_manager() {
    assert!(Harness::new(false).manager.is_none());
    assert!(Harness::new(true).manager.is_some());
}

#[test]
fn one_request_carries_every_value_in_order() {
    let mut harness = Harness::new(true);
    let request = harness.request();
    request.add_value(sealed(b"alice").as_fd(), Kind::Username, 1);
    request.add_value(sealed(b"hunter2").as_fd(), Kind::Password, 0);
    request.commit();
    harness.roundtrip().expect("roundtrip");
    let seen = harness.seen.lock().expect("seen");
    assert_eq!(seen.committed.len(), 1);
    let (window, values) = &seen.committed[0];
    assert_eq!(window, "window-1");
    assert_eq!(
        values,
        &[
            ("alice".to_owned(), ValueKind::Username, true),
            ("hunter2".to_owned(), ValueKind::Password, false),
        ]
    );
}

#[test]
fn a_bad_value_is_refused_not_a_protocol_error() {
    for bad in [
        value(b"hunter2", SealFlags::empty()),
        sealed(&[b'x'; 4097]),
        sealed(b"two\nlines"),
    ] {
        let mut harness = Harness::new(true);
        let request = harness.request();
        request.add_value(sealed(b"alice").as_fd(), Kind::Username, 1);
        request.add_value(bad.as_fd(), Kind::Password, 0);
        request.commit();
        harness.roundtrip().expect("roundtrip");
        assert_eq!(harness.client.outcomes, [client_request::Outcome::Refused]);
        assert!(harness.seen.lock().expect("seen").committed.is_empty());
    }
}

#[test]
fn destroying_a_committed_request_withdraws_it() {
    let mut harness = Harness::new(true);
    let committed = harness.request();
    committed.add_value(sealed(b"hunter2").as_fd(), Kind::Password, 0);
    committed.commit();
    let uncommitted = harness.request();
    uncommitted.add_value(sealed(b"hunter2").as_fd(), Kind::Password, 0);
    harness.roundtrip().expect("roundtrip");
    committed.destroy();
    uncommitted.destroy();
    harness.roundtrip().expect("roundtrip");
    assert_eq!(harness.seen.lock().expect("seen").withdrawn, 1);
}

#[test]
fn a_commit_without_a_value_is_a_protocol_error() {
    let mut harness = Harness::new(true);
    let request = harness.request();
    request.commit();
    expect_error(&mut harness, request.id().protocol_id(), NO_VALUE);
}

#[test]
fn a_value_after_the_commit_is_a_protocol_error() {
    let mut harness = Harness::new(true);
    let request = harness.request();
    request.add_value(sealed(b"hunter2").as_fd(), Kind::Password, 0);
    request.commit();
    request.add_value(sealed(b"more").as_fd(), Kind::Password, 0);
    expect_error(&mut harness, request.id().protocol_id(), ALREADY_COMMITTED);
}

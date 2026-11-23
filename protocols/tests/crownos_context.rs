//! Drives `crownos_context_v1` from real clients over socket pairs: each
//! client sees what its scope allows, and a revoke cuts a live client.

use std::{
    collections::VecDeque,
    error::Error,
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crownos_protocols::context::v1::{
    client::{
        crownos_context_manager_v1::{self as client_manager, CrownosContextManagerV1},
        crownos_context_window_v1::{self as client_window, CrownosContextWindowV1},
    },
    server::{
        crownos_context_manager_v1::CrownosContextManagerV1 as ServerManager,
        crownos_context_window_v1::CrownosContextWindowV1 as ServerWindow,
    },
};
use protocols::crownos_context::{
    BindDecision, ContentType, ContextGlobalData, ContextHandler, ContextRecord, ContextState,
    ContextWindowFlags, FlagReason, PauseOrigin, Placement, RevokeReason, Sandbox, Scope,
    Sensitivity, WindowKey,
};
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::WlRegistry,
};
use wayland_server::{
    Client, Display, DisplayHandle,
    backend::{ClientData, ClientId},
    delegate_dispatch, delegate_global_dispatch,
};

type TestResult = Result<(), Box<dyn Error>>;
type Command = Box<dyn FnOnce(&mut Server, &DisplayHandle) + Send>;

/// A compositor that answers binds in order from `decisions`.
struct Server {
    context: ContextState,
    decisions: VecDeque<BindDecision>,
    clients: Vec<ClientId>,
    flags: u32,
    global_pauses: Vec<(bool, PauseOrigin)>,
}

impl ContextHandler for Server {
    fn context_state(&mut self) -> &mut ContextState {
        &mut self.context
    }

    fn context_bind_decision(&mut self, client: &Client) -> BindDecision {
        self.clients.push(client.id());
        self.decisions.pop_front().unwrap_or(BindDecision::Refused)
    }

    fn set_context_paused(&mut self, paused: bool, origin: PauseOrigin) {
        self.global_pauses.push((paused, origin));
        self.context.set_paused(paused, origin);
    }

    fn flag_window_sensitive(&mut self, _key: WindowKey, _reason: FlagReason, _duration_ms: u32) {
        self.flags += 1;
    }

    fn context_client_gone(&mut self, _client: Option<ClientId>) {}
}

delegate_global_dispatch!(Server: [ServerManager: ContextGlobalData] => ContextState);
delegate_dispatch!(Server: [ServerManager: ()] => ContextState);
delegate_dispatch!(Server: [ServerWindow: WindowKey] => ContextState);

struct TestClient;
impl ClientData for TestClient {}

/// What one client heard, one line per event.
#[derive(Default)]
struct Recorder {
    heard: Vec<String>,
    windows: Vec<CrownosContextWindowV1>,
}

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for Recorder {
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

impl wayland_client::Dispatch<CrownosContextManagerV1, ()> for Recorder {
    fn event(
        recorder: &mut Self,
        _: &CrownosContextManagerV1,
        event: client_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let line = match event {
            client_manager::Event::Window { window, identifier } => {
                recorder.windows.push(window);
                format!("window {identifier}")
            }
            client_manager::Event::Scope { scope } => format!("scope {}", u32::from(scope)),
            client_manager::Event::Revoked { .. } => "revoked".to_owned(),
            client_manager::Event::Paused { paused, .. } => format!("paused {paused}"),
            client_manager::Event::Session { .. } => "session".to_owned(),
            client_manager::Event::Focus { .. } => "focus".to_owned(),
            client_manager::Event::Done => "done".to_owned(),
            _ => "other".to_owned(),
        };
        recorder.heard.push(line);
    }

    wayland_client::event_created_child!(Recorder, CrownosContextManagerV1, [
        client_manager::EVT_WINDOW_OPCODE => (CrownosContextWindowV1, ()),
    ]);
}

impl wayland_client::Dispatch<CrownosContextWindowV1, ()> for Recorder {
    fn event(
        recorder: &mut Self,
        _: &CrownosContextWindowV1,
        event: client_window::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            client_window::Event::Title { title } => recorder.heard.push(format!("title {title}")),
            client_window::Event::Closed => recorder.heard.push("closed".to_owned()),
            _ => {}
        }
    }
}

/// One client connection with its own queue.
struct TestConnection {
    globals: wayland_client::globals::GlobalList,
    queue: EventQueue<Recorder>,
    recorder: Recorder,
    manager: CrownosContextManagerV1,
}

impl TestConnection {
    fn bind(socket: UnixStream, version: u32) -> Result<Self, Box<dyn Error>> {
        let connection = Connection::from_socket(socket)?;
        let (globals, mut queue) = registry_queue_init::<Recorder>(&connection)?;
        let manager = globals.bind(&queue.handle(), version..=version, ())?;
        let mut recorder = Recorder::default();
        queue.roundtrip(&mut recorder)?;
        Ok(Self {
            globals,
            queue,
            recorder,
            manager,
        })
    }

    fn roundtrip(&mut self) -> TestResult {
        self.queue.roundtrip(&mut self.recorder)?;
        Ok(())
    }

    fn take_heard(&mut self) -> Result<Vec<String>, Box<dyn Error>> {
        self.roundtrip()?;
        Ok(std::mem::take(&mut self.recorder.heard))
    }
}

/// A compositor on its own thread, stopped when dropped. Commands run on
/// its thread between two dispatches.
struct Harness {
    running: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
    commands: Sender<(Command, Sender<()>)>,
}

impl Harness {
    fn start(sockets: Vec<UnixStream>, decisions: Vec<BindDecision>) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let (commands, received) = channel();
        let server = {
            let running = Arc::clone(&running);
            thread::spawn(move || serve(sockets, decisions.into(), &received, &running))
        };
        Self {
            running,
            server: Some(server),
            commands,
        }
    }

    fn run(
        &self,
        command: impl FnOnce(&mut Server, &DisplayHandle) + Send + 'static,
    ) -> TestResult {
        let (done, finished) = channel();
        self.commands.send((Box::new(command), done))?;
        finished.recv_timeout(Duration::from_secs(5))?;
        Ok(())
    }

    fn sync(&self, records: Vec<ContextRecord>) -> TestResult {
        self.run(move |server, display| {
            server.context.sync::<Server>(display, &records, None, 0);
        })
    }

    fn set_scope(&self, client: usize, scope: Option<Scope>) -> TestResult {
        self.run(move |server, display| {
            if let Some(id) = server.clients.get(client).cloned() {
                server
                    .context
                    .set_scope::<Server>(display, &id, scope, RevokeReason::User);
            }
        })
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

fn serve(
    sockets: Vec<UnixStream>,
    decisions: VecDeque<BindDecision>,
    commands: &Receiver<(Command, Sender<()>)>,
    running: &AtomicBool,
) {
    let Ok(mut display) = Display::<Server>::new() else {
        return;
    };
    let mut handle = display.handle();
    let mut server = Server {
        context: ContextState::new::<Server, _>(&handle, |_| true),
        decisions,
        clients: Vec::new(),
        flags: 0,
        global_pauses: Vec::new(),
    };
    for socket in sockets {
        let _ = handle.insert_client(socket, Arc::new(TestClient));
    }
    while running.load(Ordering::Acquire) {
        let _ = display.dispatch_clients(&mut server);
        while let Ok((command, done)) = commands.try_recv() {
            command(&mut server, &handle);
            let _ = display.flush_clients();
            let _ = done.send(());
        }
        let _ = display.flush_clients();
        thread::sleep(Duration::from_millis(1));
    }
}

fn record(key: WindowKey, title: &str) -> ContextRecord {
    ContextRecord {
        key,
        identifier: format!("w{key}"),
        app_id: "org.example.Notes".to_owned(),
        title: title.to_owned(),
        pid: 0,
        sandbox: Sandbox::default(),
        placement: Placement {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            output: "DP-1".to_owned(),
            workspace: "1".to_owned(),
        },
        flags: ContextWindowFlags::Visible,
        content_type: ContentType::None,
        sensitivity: Sensitivity::empty(),
    }
}

/// Connects clients one at a time, so they bind in the order of `decisions`.
fn connect(
    decisions: Vec<(BindDecision, u32)>,
) -> Result<(Harness, Vec<TestConnection>), Box<dyn Error>> {
    let mut server_sockets = Vec::new();
    let mut client_sockets = Vec::new();
    for _ in &decisions {
        let (server, client) = UnixStream::pair()?;
        server_sockets.push(server);
        client_sockets.push(client);
    }
    let harness = Harness::start(
        server_sockets,
        decisions.iter().map(|(decision, _)| *decision).collect(),
    );
    let mut connections = Vec::new();
    for (socket, (_, version)) in client_sockets.into_iter().zip(&decisions) {
        connections.push(TestConnection::bind(socket, *version)?);
    }
    Ok((harness, connections))
}

const TEXT: Scope = Scope::Metadata.union(Scope::Text);

#[test]
fn each_client_sees_what_its_scope_allows() -> TestResult {
    let (harness, mut clients) = connect(vec![
        (BindDecision::Granted(TEXT), 2),
        (BindDecision::Granted(Scope::Metadata), 2),
        (BindDecision::Refused, 2),
    ])?;
    harness.sync(vec![record(1, "Secret plan")])?;

    let with_text = clients[0].take_heard()?;
    assert!(with_text.contains(&"window w1".to_owned()));
    assert!(with_text.contains(&"title Secret plan".to_owned()));

    let metadata_only = clients[1].take_heard()?;
    assert_eq!(
        metadata_only[0],
        format!("scope {}", Scope::Metadata.bits())
    );
    assert!(metadata_only.contains(&"window w1".to_owned()));
    assert!(!metadata_only.iter().any(|line| line.starts_with("title")));

    assert_eq!(clients[2].take_heard()?, ["revoked"]);
    Ok(())
}

#[test]
fn a_revoke_cuts_a_live_client_and_leaves_the_others() -> TestResult {
    let (harness, mut clients) = connect(vec![
        (BindDecision::Granted(TEXT), 2),
        (BindDecision::Granted(TEXT), 2),
    ])?;
    harness.sync(vec![record(1, "Inbox")])?;
    clients[0].take_heard()?;
    clients[1].take_heard()?;

    harness.set_scope(0, None)?;
    assert_eq!(clients[0].take_heard()?, ["closed", "revoked"]);

    harness.sync(vec![record(1, "Inbox"), record(2, "Draft")])?;
    assert!(clients[0].take_heard()?.is_empty());
    assert!(clients[1].take_heard()?.contains(&"window w2".to_owned()));

    clients[0].manager.set_paused(1);
    clients[0].roundtrip()?;
    harness.run(|server, _| assert!(server.global_pauses.is_empty()))?;
    Ok(())
}

#[test]
fn a_waiting_client_gets_nothing_until_it_is_granted() -> TestResult {
    let (harness, mut clients) = connect(vec![(BindDecision::Pending, 2)])?;
    harness.sync(vec![record(1, "Inbox")])?;
    assert!(clients[0].take_heard()?.is_empty());

    harness.set_scope(0, Some(Scope::Metadata))?;
    let heard = clients[0].take_heard()?;
    assert_eq!(heard.first().map(String::as_str), Some("scope 1"));
    assert!(heard.contains(&"window w1".to_owned()));
    assert_eq!(heard.last().map(String::as_str), Some("done"));
    Ok(())
}

#[test]
fn gaining_text_sends_the_titles() -> TestResult {
    let (harness, mut clients) = connect(vec![(BindDecision::Granted(Scope::Metadata), 2)])?;
    harness.sync(vec![record(1, "Inbox")])?;
    clients[0].take_heard()?;

    harness.set_scope(0, Some(TEXT))?;
    assert_eq!(clients[0].take_heard()?, ["scope 3", "title Inbox"]);
    Ok(())
}

#[test]
fn a_client_without_engine_or_pause_pauses_only_itself() -> TestResult {
    let (harness, mut clients) = connect(vec![
        (BindDecision::Granted(Scope::Metadata), 2),
        (BindDecision::Granted(Scope::Metadata), 2),
    ])?;
    harness.sync(vec![record(1, "Inbox")])?;
    clients[0].take_heard()?;
    clients[1].take_heard()?;

    clients[0].manager.set_paused(1);
    assert_eq!(clients[0].take_heard()?, ["closed", "paused 1"]);
    harness.run(|server, _| assert!(server.global_pauses.is_empty()))?;

    harness.sync(vec![record(1, "Inbox"), record(2, "Draft")])?;
    assert!(clients[0].take_heard()?.is_empty());
    assert!(clients[1].take_heard()?.contains(&"window w2".to_owned()));
    Ok(())
}

#[test]
fn a_client_with_the_pause_scope_pauses_everyone() -> TestResult {
    let (harness, mut clients) = connect(vec![
        (BindDecision::Granted(Scope::Pause), 2),
        (BindDecision::Granted(Scope::Metadata), 2),
    ])?;
    harness.sync(vec![record(1, "Inbox")])?;
    clients[1].take_heard()?;

    clients[0].manager.set_paused(1);
    clients[0].roundtrip()?;
    harness.run(|server, _| {
        assert_eq!(server.global_pauses, [(true, PauseOrigin::Client)]);
    })?;
    assert_eq!(clients[1].take_heard()?, ["closed", "paused 1"]);
    Ok(())
}

#[test]
fn flags_without_the_engine_scope_are_rate_limited() -> TestResult {
    let (harness, mut clients) = connect(vec![(BindDecision::Granted(Scope::Metadata), 2)])?;
    harness.sync(vec![record(1, "Inbox")])?;
    clients[0].take_heard()?;
    let window = clients[0]
        .recorder
        .windows
        .first()
        .cloned()
        .ok_or("no window object")?;
    window.flag_sensitive(client_window::FlagReason::SecretText, 1000);
    window.flag_sensitive(client_window::FlagReason::SecretText, 1000);
    clients[0].roundtrip()?;
    harness.run(|server, _| assert_eq!(server.flags, 1))?;
    assert!(window.is_alive());
    Ok(())
}

#[test]
fn a_version_1_client_needs_the_engine_scope() -> TestResult {
    let (harness, mut clients) = connect(vec![
        (BindDecision::Granted(Scope::Metadata), 1),
        (BindDecision::Granted(Scope::Engine | TEXT), 1),
    ])?;
    harness.sync(vec![record(1, "Inbox")])?;
    assert_eq!(clients[0].take_heard()?, ["done"]);
    let engine = clients[1].take_heard()?;
    assert!(engine.contains(&"window w1".to_owned()));
    assert!(!engine.iter().any(|line| line.starts_with("scope")));
    Ok(())
}

#[test]
fn a_revoked_manager_does_not_hide_a_waiting_one_of_the_same_client() -> TestResult {
    let (server_socket, client_socket) = UnixStream::pair()?;
    let harness = Harness::start(
        vec![server_socket],
        vec![BindDecision::Refused, BindDecision::Pending],
    );
    let mut client = TestConnection::bind(client_socket, 2)?;
    assert_eq!(client.take_heard()?, ["revoked"]);
    let second: CrownosContextManagerV1 = client.globals.bind(&client.queue.handle(), 2..=2, ())?;
    client.roundtrip()?;
    harness.run(|server, _| {
        let id = server.clients[0].clone();
        assert!(server.context.is_waiting(&id), "the pending manager waits");
    })?;
    harness.set_scope(0, Some(Scope::Metadata))?;
    harness.run(|server, _| {
        let id = server.clients[0].clone();
        assert!(!server.context.is_waiting(&id));
        assert_eq!(server.context.scope_of(&id), Some(Scope::Metadata));
    })?;
    drop(second);
    Ok(())
}

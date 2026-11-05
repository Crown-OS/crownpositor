//! Drives `crownos-android-shell-v1` from real clients over socket pairs: who
//! sees the global, the protocol errors, and the double-buffered details.
//!
//! The server is smithay's own compositor and xdg-shell, so commits run the
//! same pre-commit hooks they run in the compositor. Each client carries fake
//! client data with a fake security context, which is all the global filter
//! looks at.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crownos_protocols::android_shell::v1::{
    client::{
        crownos_android_shell_manager_v1::CrownosAndroidShellManagerV1,
        crownos_android_toplevel_v1::{CrownosAndroidToplevelV1, ResizeLock as ClientResizeLock},
    },
    server::{
        crownos_android_shell_manager_v1::CrownosAndroidShellManagerV1 as ServerManager,
        crownos_android_toplevel_v1::CrownosAndroidToplevelV1 as ServerToplevel,
    },
};
use protocols::crownos_android_shell::{
    AndroidDetails, AndroidPeer, AndroidShellGlobalData, AndroidShellHandler, AndroidShellState,
    ResizeLock, SizeLock, TitleColor,
};
use smithay::{
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel as ServerXdgToplevel,
    utils::Serial,
    wayland::{
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        security_context::SecurityContext,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
    },
};
use wayland_client::{
    Connection, DispatchError, EventQueue, Proxy, QueueHandle,
    backend::WaylandError,
    globals::{GlobalList, GlobalListContents, registry_queue_init},
    protocol::{wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_surface::WlSurface},
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel, xdg_wm_base::XdgWmBase,
};
use wayland_server::{
    Client, Display,
    backend::{ClientData, ClientId},
    delegate_dispatch, delegate_global_dispatch,
    protocol::{wl_seat::WlSeat, wl_surface::WlSurface as ServerSurface},
};

const WAYDROID_ENGINE: &str = "id.waydro.waydroid";
const MANAGER_INTERFACE: &str = "crownos_android_shell_manager_v1";
const ALREADY_EXTENDED: u32 = 0;
const NOT_ALLOWED: u32 = 1;

/// Who a test client connects as, which is what decides what it may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Peer {
    /// Through Waydroid's security context, created by the daemon.
    Hwcomposer,
    /// Unsandboxed, running the daemon's executable.
    Daemon,
    /// Through some other sandbox engine's security context.
    SandboxedApp,
}

/// Client data shaped like the compositor's: a security context when the
/// client came through one, and what its executable turned out to be.
struct FakeClient {
    compositor: CompositorClientState,
    security_context: Option<SecurityContext>,
    runs_daemon: bool,
}

impl ClientData for FakeClient {}

/// The global filter under test, written against the fake data the way the
/// compositor's is written against its own.
fn peer_of(client: &Client) -> Option<AndroidPeer> {
    let data = client.get_data::<FakeClient>()?;
    match &data.security_context {
        Some(context) => (context.sandbox_engine.as_deref() == Some(WAYDROID_ENGINE))
            .then_some(AndroidPeer::Hwcomposer),
        None => data.runs_daemon.then_some(AndroidPeer::Daemon),
    }
}

/// What the compositor heard about each toplevel, in order.
type Changes = Arc<Mutex<Vec<Option<AndroidDetails>>>>;

struct Server {
    compositor: CompositorState,
    xdg_shell: XdgShellState,
    android_shell: AndroidShellState,
    changes: Changes,
}

impl CompositorHandler for Server {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<FakeClient>()
            .expect("every test client carries fake data")
            .compositor
    }

    fn commit(&mut self, _surface: &ServerSurface) {}
}

impl XdgShellHandler for Server {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        surface.send_configure();
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }
}

impl AndroidShellHandler for Server {
    fn android_shell_state(&mut self) -> &mut AndroidShellState {
        &mut self.android_shell
    }

    fn android_details_changed(
        &mut self,
        _toplevel: &ServerXdgToplevel,
        details: Option<&AndroidDetails>,
    ) {
        self.changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.cloned());
    }
}

smithay::delegate_dispatch2!(Server);
delegate_global_dispatch!(Server: [ServerManager: AndroidShellGlobalData] => AndroidShellState);
delegate_dispatch!(Server: [ServerManager: Option<AndroidPeer>] => AndroidShellState);
delegate_dispatch!(Server: [ServerToplevel: ()] => AndroidShellState);

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

wayland_client::delegate_noop!(ClientState: ignore WlCompositor);
wayland_client::delegate_noop!(ClientState: ignore WlSurface);
wayland_client::delegate_noop!(ClientState: ignore XdgWmBase);
wayland_client::delegate_noop!(ClientState: ignore XdgSurface);
wayland_client::delegate_noop!(ClientState: ignore XdgToplevel);
wayland_client::delegate_noop!(ClientState: ignore CrownosAndroidShellManagerV1);
wayland_client::delegate_noop!(ClientState: ignore CrownosAndroidToplevelV1);

/// A compositor on its own thread with one client under test, stopped when
/// dropped.
struct Harness {
    running: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
    queue: EventQueue<ClientState>,
    globals: GlobalList,
    changes: Changes,
}

/// A window with its surface, ready to be extended.
struct Window {
    surface: WlSurface,
    toplevel: XdgToplevel,
}

impl Harness {
    fn new(connects: Peer) -> Self {
        let (server_socket, client_socket) = UnixStream::pair().expect("socket pair");
        let running = Arc::new(AtomicBool::new(true));
        let changes = Changes::default();
        let server = {
            let (running, changes) = (running.clone(), changes.clone());
            thread::spawn(move || serve(server_socket, connects, &running, changes))
        };
        let connection = Connection::from_socket(client_socket).expect("client connection");
        let (globals, queue) = registry_queue_init::<ClientState>(&connection).expect("registry");
        Self {
            running,
            server: Some(server),
            queue,
            globals,
            changes,
        }
    }

    fn sees_the_manager(&self) -> bool {
        self.globals.contents().with_list(|globals| {
            globals
                .iter()
                .any(|global| global.interface == MANAGER_INTERFACE)
        })
    }

    fn manager(&self) -> CrownosAndroidShellManagerV1 {
        self.globals
            .bind(&self.queue.handle(), 1..=1, ())
            .expect("the android shell manager")
    }

    fn window(&self) -> Window {
        let handle = self.queue.handle();
        let compositor: WlCompositor = self.globals.bind(&handle, 1..=6, ()).expect("compositor");
        let wm_base: XdgWmBase = self.globals.bind(&handle, 1..=6, ()).expect("xdg_wm_base");
        let surface = compositor.create_surface(&handle, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &handle, ());
        let toplevel = xdg_surface.get_toplevel(&handle, ());
        Window { surface, toplevel }
    }

    fn extend(
        &self,
        manager: &CrownosAndroidShellManagerV1,
        window: &Window,
    ) -> CrownosAndroidToplevelV1 {
        manager.get_android_toplevel(&window.toplevel, &self.queue.handle(), ())
    }

    /// The protocol error the client was disconnected with, if any, as the
    /// object's protocol id and the error code.
    fn error(&mut self) -> Option<(u32, u32)> {
        match self.queue.roundtrip(&mut ClientState) {
            Ok(_) => None,
            Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
                Some((error.object_id, error.code))
            }
            Err(other) => panic!("unexpected failure: {other}"),
        }
    }

    fn changes(&self) -> Vec<Option<AndroidDetails>> {
        self.changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

fn fake_client(security_context: Option<SecurityContext>, runs_daemon: bool) -> Arc<FakeClient> {
    Arc::new(FakeClient {
        compositor: CompositorClientState::default(),
        security_context,
        runs_daemon,
    })
}

fn context(engine: &str, creator: ClientId) -> SecurityContext {
    SecurityContext {
        sandbox_engine: Some(engine.to_owned()),
        app_id: Some("waydroid".to_owned()),
        instance_id: None,
        creator_client_id: creator,
    }
}

fn serve(socket: UnixStream, connects: Peer, running: &AtomicBool, changes: Changes) {
    let mut display = Display::<Server>::new().expect("display");
    let mut handle = display.handle();
    let mut state = Server {
        compositor: CompositorState::new::<Server>(&handle),
        xdg_shell: XdgShellState::new::<Server>(&handle),
        android_shell: AndroidShellState::new::<Server, _>(&handle, peer_of),
        changes,
    };

    // The daemon that created the security contexts. Its socket stays open
    // for as long as the server runs, so the context's creator is live.
    let (daemon_socket, _daemon_peer) = UnixStream::pair().expect("daemon socket pair");
    let daemon = handle
        .insert_client(daemon_socket, fake_client(None, true))
        .expect("insert the daemon");
    let data = match connects {
        Peer::Hwcomposer => fake_client(Some(context(WAYDROID_ENGINE, daemon.id())), false),
        Peer::Daemon => fake_client(None, true),
        Peer::SandboxedApp => fake_client(Some(context("org.flatpak", daemon.id())), false),
    };
    handle
        .insert_client(socket, data)
        .expect("insert the client");

    while running.load(Ordering::Acquire) {
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn the_runtime_sees_the_manager() {
    assert!(Harness::new(Peer::Hwcomposer).sees_the_manager());
    assert!(Harness::new(Peer::Daemon).sees_the_manager());
}

#[test]
fn an_ordinary_sandboxed_client_does_not() {
    assert!(!Harness::new(Peer::SandboxedApp).sees_the_manager());
}

#[test]
fn a_toplevel_is_extended_once_at_most() {
    let mut harness = Harness::new(Peer::Hwcomposer);
    let manager = harness.manager();
    let window = harness.window();
    harness.extend(&manager, &window);
    assert_eq!(harness.error(), None);

    harness.extend(&manager, &window);
    let manager = manager.id().protocol_id();
    assert_eq!(harness.error(), Some((manager, ALREADY_EXTENDED)));
}

#[test]
fn a_destroyed_extension_can_be_replaced() {
    let mut harness = Harness::new(Peer::Hwcomposer);
    let manager = harness.manager();
    let window = harness.window();
    harness.extend(&manager, &window).destroy();
    harness.extend(&manager, &window);
    assert_eq!(harness.error(), None);
}

#[test]
fn only_the_daemon_marks_placeholders() {
    let mut harness = Harness::new(Peer::Hwcomposer);
    let manager = harness.manager();
    let window = harness.window();
    manager.mark_placeholder(&window.toplevel, "waydroid.org.example".into());
    let manager = manager.id().protocol_id();
    assert_eq!(harness.error(), Some((manager, NOT_ALLOWED)));

    let mut harness = Harness::new(Peer::Daemon);
    let manager = harness.manager();
    let window = harness.window();
    manager.mark_placeholder(&window.toplevel, "waydroid.org.example".into());
    assert_eq!(harness.error(), None);
}

#[test]
fn details_apply_on_the_next_commit() {
    let mut harness = Harness::new(Peer::Hwcomposer);
    let manager = harness.manager();
    let window = harness.window();
    let extension = harness.extend(&manager, &window);
    window.surface.commit();
    assert_eq!(harness.error(), None);
    assert!(harness.changes().is_empty(), "nothing was asked for yet");

    extension.set_task(42, "org.example.notes".into());
    extension.set_resize_lock(ClientResizeLock::Aspect);
    extension.set_aspect(9, 16);
    extension.set_title_color(0xff20_4060);
    assert_eq!(harness.error(), None);
    assert!(harness.changes().is_empty(), "held until the commit");

    window.surface.commit();
    assert_eq!(harness.error(), None);
    let changes = harness.changes();
    let [Some(details)] = changes.as_slice() else {
        panic!("one change with details, got {changes:?}");
    };
    assert_eq!(details.task.as_ref().map(|task| task.id), Some(42));
    assert_eq!(
        details.task.as_ref().map(|task| task.package.as_str()),
        Some("org.example.notes")
    );
    assert_eq!(details.resize_lock, ResizeLock::Aspect);
    assert!(matches!(details.size_lock(), SizeLock::Aspect(ratio)
        if ratio.width.get() == 9 && ratio.height.get() == 16));
    assert_eq!(details.title_color, TitleColor::new(0xff20_4060));

    window.surface.commit();
    assert_eq!(harness.error(), None);
    assert_eq!(
        harness.changes().len(),
        1,
        "an empty commit changes nothing"
    );
}

#[test]
fn destroying_the_extension_drops_the_details() {
    let mut harness = Harness::new(Peer::Hwcomposer);
    let manager = harness.manager();
    let window = harness.window();
    let extension = harness.extend(&manager, &window);
    extension.set_resize_lock(ClientResizeLock::Fixed);
    window.surface.commit();
    extension.destroy();
    window.surface.commit();
    assert_eq!(harness.error(), None);

    let changes = harness.changes();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert_eq!(
        changes[0].as_ref().map(AndroidDetails::size_lock),
        Some(SizeLock::Fixed)
    );
    assert_eq!(changes[1], None);
}

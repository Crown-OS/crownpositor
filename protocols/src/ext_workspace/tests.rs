use std::{os::unix::net::UnixStream, sync::Arc};

use smithay::output::{Output, PhysicalProperties, Subpixel};
use wayland_client::{
    Connection, Dispatch as ClientDispatch, EventQueue, QueueHandle,
    protocol::wl_registry::{self, WlRegistry},
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{
        self as group_client, ExtWorkspaceGroupHandleV1 as ClientGroup,
    },
    ext_workspace_handle_v1::{self as workspace_client, ExtWorkspaceHandleV1 as ClientWorkspace},
    ext_workspace_manager_v1::{self as manager_client, ExtWorkspaceManagerV1 as ClientManager},
};
use wayland_server::{Display, delegate_dispatch, delegate_global_dispatch};

use super::*;

struct Server {
    state: ExtWorkspaceState,
    activated: Vec<u64>,
}

impl ExtWorkspaceHandler for Server {
    fn ext_workspace_state(&mut self) -> &mut ExtWorkspaceState {
        &mut self.state
    }

    fn activate_workspace(&mut self, key: u64) {
        self.activated.push(key);
    }
}

delegate_global_dispatch!(Server: [ExtWorkspaceManagerV1: ExtWorkspaceGlobalData] => ExtWorkspaceState);
delegate_dispatch!(Server: [ExtWorkspaceManagerV1: ManagerData] => ExtWorkspaceState);
delegate_dispatch!(Server: [ExtWorkspaceGroupHandleV1: u64] => ExtWorkspaceState);
delegate_dispatch!(Server: [ExtWorkspaceHandleV1: WorkspaceData] => ExtWorkspaceState);

#[derive(Default)]
struct Seen {
    manager: Option<ClientManager>,
    workspaces: Vec<(ClientWorkspace, Option<String>, Option<String>, bool, bool)>,
    groups: usize,
    entered: usize,
    dones: usize,
}

impl ClientDispatch<WlRegistry, ()> for Seen {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
            && interface == "ext_workspace_manager_v1"
        {
            state.manager = Some(registry.bind(name, 1, queue, ()));
        }
    }
}

impl ClientDispatch<ClientManager, ()> for Seen {
    fn event(
        state: &mut Self,
        _: &ClientManager,
        event: manager_client::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager_client::Event::Workspace { workspace } => {
                state.workspaces.push((workspace, None, None, false, false))
            }
            manager_client::Event::WorkspaceGroup { .. } => state.groups += 1,
            manager_client::Event::Done => state.dones += 1,
            _ => {}
        }
    }

    wayland_client::event_created_child!(Seen, ClientManager, [
        manager_client::EVT_WORKSPACE_GROUP_OPCODE => (ClientGroup, ()),
        manager_client::EVT_WORKSPACE_OPCODE => (ClientWorkspace, ()),
    ]);
}

impl ClientDispatch<ClientGroup, ()> for Seen {
    fn event(
        state: &mut Self,
        _: &ClientGroup,
        event: group_client::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let group_client::Event::WorkspaceEnter { .. } = event {
            state.entered += 1;
        }
    }
}

impl ClientDispatch<ClientWorkspace, ()> for Seen {
    fn event(
        state: &mut Self,
        workspace: &ClientWorkspace,
        event: workspace_client::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state
            .workspaces
            .iter_mut()
            .find(|entry| entry.0 == *workspace)
        else {
            return;
        };
        match event {
            workspace_client::Event::Id { id } => entry.1 = Some(id),
            workspace_client::Event::Name { name } => entry.2 = Some(name),
            workspace_client::Event::State { state } => {
                entry.3 = state
                    .into_result()
                    .is_ok_and(|state| state.contains(workspace_client::State::Active))
            }
            workspace_client::Event::Removed => entry.4 = true,
            _ => {}
        }
    }
}

struct Harness {
    display: Display<Server>,
    server: Server,
    queue: EventQueue<Seen>,
    seen: Seen,
}

impl Harness {
    fn new() -> Self {
        let mut display = Display::<Server>::new().unwrap();
        let state = ExtWorkspaceState::new::<Server, _>(&display.handle(), |_| true);
        let (server_end, client_end) = UnixStream::pair().unwrap();
        display
            .handle()
            .insert_client(server_end, Arc::new(()))
            .unwrap();
        let connection = Connection::from_socket(client_end).unwrap();
        let queue = connection.new_event_queue();
        connection.display().get_registry(&queue.handle(), ());
        let mut harness = Self {
            display,
            server: Server {
                state,
                activated: Vec::new(),
            },
            queue,
            seen: Seen::default(),
        };
        harness.pump();
        harness.pump();
        harness
    }

    fn pump(&mut self) {
        for _ in 0..3 {
            self.queue.flush().unwrap();
            self.display.dispatch_clients(&mut self.server).unwrap();
            self.display.flush_clients().unwrap();
            if let Some(guard) = self.queue.prepare_read() {
                let _ = guard.read();
            }
            self.queue.dispatch_pending(&mut self.seen).unwrap();
        }
    }

    fn update(&mut self, groups: Vec<WorkspaceGroup>, workspaces: Vec<WorkspaceInfo>) {
        let handle = self.display.handle();
        self.server
            .state
            .update::<Server>(&handle, groups, workspaces);
        self.pump();
    }
}

fn output() -> Output {
    Output::new(
        "test".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: String::new(),
            model: String::new(),
            serial_number: String::new(),
        },
    )
}

fn workspace(key: u64, active: bool) -> WorkspaceInfo {
    WorkspaceInfo {
        key,
        group: 1,
        id: format!("ws-{key}"),
        name: key.to_string(),
        coordinates: vec![key as u32],
        active,
    }
}

#[test]
fn clients_see_new_changed_and_removed_workspaces() {
    let mut harness = Harness::new();
    let group = WorkspaceGroup {
        key: 1,
        output: output(),
    };

    harness.update(
        vec![group.clone()],
        vec![workspace(10, true), workspace(11, false)],
    );
    assert_eq!(harness.seen.groups, 1);
    assert_eq!(harness.seen.entered, 2);
    assert_eq!(harness.seen.workspaces.len(), 2);
    assert_eq!(harness.seen.workspaces[0].1.as_deref(), Some("ws-10"));
    assert!(harness.seen.workspaces[0].3);
    let dones = harness.seen.dones;

    harness.update(
        vec![group.clone()],
        vec![workspace(10, true), workspace(11, false)],
    );
    assert_eq!(
        harness.seen.dones, dones,
        "an unchanged model sends nothing"
    );

    harness.update(vec![group], vec![workspace(10, false), workspace(11, true)]);
    assert!(!harness.seen.workspaces[0].3);
    assert!(harness.seen.workspaces[1].3);

    harness.update(Vec::new(), Vec::new());
    assert!(harness.seen.workspaces.iter().all(|entry| entry.4));
}

#[test]
fn activation_waits_for_commit() {
    let mut harness = Harness::new();
    let group = WorkspaceGroup {
        key: 1,
        output: output(),
    };
    harness.update(vec![group], vec![workspace(10, true), workspace(11, false)]);

    harness.seen.workspaces[1].0.activate();
    harness.pump();
    assert!(harness.server.activated.is_empty());

    harness.seen.manager.as_ref().unwrap().commit();
    harness.pump();
    assert_eq!(harness.server.activated, vec![11]);
}

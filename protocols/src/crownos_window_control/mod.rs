//! Server-side `crownos_window_control_v1`: the window list for the agent
//! daemon, and its requests.
//!
//! The compositor hands this module one [`WindowRecord`] per mapped window
//! every loop iteration through [`WindowControlState::sync`]; the diff against
//! what each bound client was last told becomes `window`, property, `done`,
//! `closed` and `focus` events. Requests come back through
//! [`WindowControlHandler`] keyed by the compositor's own window key.

mod dispatch;

use std::collections::HashMap;

use crownos_protocols::window_control::v1::server::{
    crownos_window_control_manager_v1::CrownosWindowControlManagerV1,
    crownos_window_v1::CrownosWindowV1,
};
use wayland_server::{Client, DisplayHandle, Resource, backend::GlobalId};

pub use crownos_protocols::window_control::v1::server::crownos_window_v1::State as WindowFlags;
pub use dispatch::WindowControlDispatch;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// The compositor's own name for a window, stable for its lifetime.
pub type WindowKey = u64;

/// Where a window is: its content rectangle in its output's logical space,
/// the output's name and its workspace's `ext_workspace` identifier.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub output: String,
    pub workspace: String,
}

/// One mapped window as the protocol describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRecord {
    pub key: WindowKey,
    /// Shared with the window's `ext_foreign_toplevel_handle_v1`.
    pub identifier: String,
    pub app_id: String,
    pub title: String,
    /// 0 when unknown.
    pub pid: i32,
    pub placement: Placement,
    pub flags: WindowFlags,
}

/// What the compositor provides for this protocol to be delegated to
/// [`WindowControlState`].
pub trait WindowControlHandler {
    fn window_control_state(&mut self) -> &mut WindowControlState;
    fn focus_window(&mut self, key: WindowKey);
    fn close_window(&mut self, key: WindowKey);
    fn move_window_to_workspace(&mut self, key: WindowKey, workspace: &str);
    fn make_window_visible(&mut self, key: WindowKey);
    fn make_window_hidden(&mut self, key: WindowKey);
    fn set_window_rectangle(&mut self, key: WindowKey, x: i32, y: i32, width: i32, height: i32);
    /// The `ext_workspace` identifier of the agent workspace, created on demand;
    /// empty when the compositor cannot provide one.
    fn agent_workspace_identifier(&mut self) -> String;
}

/// Global data: which clients may see the manager.
pub struct WindowControlGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

struct Manager {
    resource: CrownosWindowControlManagerV1,
    windows: HashMap<WindowKey, CrownosWindowV1>,
}

/// Delegate type for the `crownos_window_control_manager_v1` global.
pub struct WindowControlState {
    global: GlobalId,
    managers: Vec<Manager>,
    known: HashMap<WindowKey, WindowRecord>,
    focus: Option<WindowKey>,
}

impl WindowControlState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: WindowControlDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosWindowControlManagerV1, _>(
            VERSION,
            WindowControlGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            managers: Vec::new(),
            known: HashMap::new(),
            focus: None,
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Whether a window the protocol announced still exists.
    pub fn knows(&self, key: WindowKey) -> bool {
        self.known.contains_key(&key)
    }

    /// Brings every bound client up to date with `records` and `focus`.
    /// Cheap when nothing changed: records are compared before anything is sent.
    pub fn sync<D: WindowControlDispatch>(
        &mut self,
        display: &DisplayHandle,
        records: &[WindowRecord],
        focus: Option<WindowKey>,
    ) {
        self.managers.retain(|manager| manager.resource.is_alive());
        let gone: Vec<WindowKey> = self
            .known
            .keys()
            .filter(|key| !records.iter().any(|record| record.key == **key))
            .copied()
            .collect();
        for key in gone {
            self.known.remove(&key);
            for manager in &mut self.managers {
                if let Some(window) = manager.windows.remove(&key) {
                    window.closed();
                }
            }
        }
        for record in records {
            match self.known.get(&record.key) {
                Some(known) if known == record => continue,
                Some(known) => {
                    for manager in &self.managers {
                        if let Some(window) = manager.windows.get(&record.key) {
                            send_changes(window, known, record);
                            window.done();
                        }
                    }
                }
                None => {
                    for manager in &mut self.managers {
                        announce::<D>(display, manager, record);
                    }
                }
            }
            self.known.insert(record.key, record.clone());
        }
        if self.focus != focus {
            self.focus = focus;
            for manager in &self.managers {
                manager
                    .resource
                    .focus(focus.and_then(|key| manager.windows.get(&key)));
            }
        }
    }

    fn bind<D: WindowControlDispatch>(
        &mut self,
        display: &DisplayHandle,
        resource: CrownosWindowControlManagerV1,
    ) {
        let mut manager = Manager {
            resource,
            windows: HashMap::new(),
        };
        for record in self.known.values() {
            announce::<D>(display, &mut manager, record);
        }
        manager
            .resource
            .focus(self.focus.and_then(|key| manager.windows.get(&key)));
        manager.resource.done();
        self.managers.push(manager);
    }

    fn unbind(&mut self, resource: &CrownosWindowControlManagerV1) {
        self.managers
            .retain(|manager| manager.resource.id() != resource.id());
    }
}

fn announce<D: WindowControlDispatch>(
    display: &DisplayHandle,
    manager: &mut Manager,
    record: &WindowRecord,
) {
    let Some(client) = manager.resource.client() else {
        return;
    };
    let Ok(window) = client.create_resource::<CrownosWindowV1, WindowKey, D>(
        display,
        manager.resource.version(),
        record.key,
    ) else {
        return;
    };
    manager.resource.window(&window);
    window.identifier(record.identifier.clone());
    window.app_id(record.app_id.clone());
    window.title(record.title.clone());
    window.process(record.pid);
    send_placement(&window, &record.placement);
    window.state(record.flags);
    window.done();
    manager.windows.insert(record.key, window);
}

fn send_changes(window: &CrownosWindowV1, known: &WindowRecord, record: &WindowRecord) {
    if known.app_id != record.app_id {
        window.app_id(record.app_id.clone());
    }
    if known.title != record.title {
        window.title(record.title.clone());
    }
    if known.pid != record.pid {
        window.process(record.pid);
    }
    if known.placement != record.placement {
        send_placement(window, &record.placement);
    }
    if known.flags != record.flags {
        window.state(record.flags);
    }
}

fn send_placement(window: &CrownosWindowV1, placement: &Placement) {
    window.placement(
        placement.x,
        placement.y,
        placement.width,
        placement.height,
        placement.output.clone(),
        placement.workspace.clone(),
    );
}

//! Server-side `ext_workspace_v1`.
//!
//! Lists workspaces to bars, pagers and screencast pickers, grouped by the
//! output they live on, and lets those clients switch to one. The compositor
//! hands over the whole model with [`ExtWorkspaceState::update`] whenever it
//! may have changed; this module works out the difference per client and sends
//! only that, so calling it every loop iteration costs a comparison.

mod dispatch;

use std::{collections::HashMap, sync::Mutex};

use smithay::output::Output;
use wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1::{ExtWorkspaceGroupHandleV1, GroupCapabilities},
    ext_workspace_handle_v1::{ExtWorkspaceHandleV1, State, WorkspaceCapabilities},
    ext_workspace_manager_v1::ExtWorkspaceManagerV1,
};
use wayland_server::{
    Client, Dispatch, DisplayHandle, GlobalDispatch, Resource, backend::GlobalId,
    protocol::wl_output::WlOutput,
};

pub use dispatch::{ExtWorkspaceGlobalData, ManagerData, WorkspaceData};

/// One output's workspaces. `key` is the compositor's own identifier for it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceGroup {
    pub key: u64,
    pub output: Output,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceInfo {
    /// The compositor's own identifier, handed back by [`ExtWorkspaceHandler::activate_workspace`].
    pub key: u64,
    /// The key of the [`WorkspaceGroup`] it belongs to.
    pub group: u64,
    /// Stable for the workspace's lifetime, and unique across the session.
    pub id: String,
    pub name: String,
    pub coordinates: Vec<u32>,
    pub active: bool,
}

pub trait ExtWorkspaceHandler {
    fn ext_workspace_state(&mut self) -> &mut ExtWorkspaceState;

    /// A client committed an activation of the workspace with this key.
    fn activate_workspace(&mut self, key: u64);
}

pub struct ExtWorkspaceState {
    global: GlobalId,
    groups: Vec<WorkspaceGroup>,
    workspaces: Vec<WorkspaceInfo>,
    instances: Vec<Instance>,
}

/// What one bound manager has been told.
struct Instance {
    manager: ExtWorkspaceManagerV1,
    groups: HashMap<u64, ExtWorkspaceGroupHandleV1>,
    workspaces: HashMap<u64, ExtWorkspaceHandleV1>,
}

impl ExtWorkspaceState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: ExtWorkspaceDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        Self {
            global: display.create_global::<D, ExtWorkspaceManagerV1, _>(
                1,
                ExtWorkspaceGlobalData::new(filter),
            ),
            groups: Vec::new(),
            workspaces: Vec::new(),
            instances: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Replaces the model, telling every client what changed.
    pub fn update<D: ExtWorkspaceDispatch>(
        &mut self,
        display: &DisplayHandle,
        groups: Vec<WorkspaceGroup>,
        workspaces: Vec<WorkspaceInfo>,
    ) {
        if groups == self.groups && workspaces == self.workspaces {
            return;
        }
        for instance in &mut self.instances {
            instance.send_diff::<D>(
                display,
                (&self.groups, &self.workspaces),
                (&groups, &workspaces),
            );
        }
        self.groups = groups;
        self.workspaces = workspaces;
    }

    /// A client bound `wl_output` after it learnt about the group on it.
    pub fn output_bound(&self, output: &Output, wl_output: &WlOutput) {
        let Some(client) = wl_output.client() else {
            return;
        };
        let keys: Vec<u64> = self
            .groups
            .iter()
            .filter(|group| group.output == *output)
            .map(|group| group.key)
            .collect();
        for instance in self
            .instances
            .iter()
            .filter(|instance| instance.manager.client().as_ref() == Some(&client))
        {
            let mut sent = false;
            for group in keys.iter().filter_map(|key| instance.groups.get(key)) {
                group.output_enter(wl_output);
                sent = true;
            }
            if sent {
                instance.manager.done();
            }
        }
    }

    fn bind<D: ExtWorkspaceDispatch>(
        &mut self,
        display: &DisplayHandle,
        manager: ExtWorkspaceManagerV1,
    ) {
        let mut instance = Instance {
            manager,
            groups: HashMap::new(),
            workspaces: HashMap::new(),
        };
        instance.send_diff::<D>(display, (&[], &[]), (&self.groups, &self.workspaces));
        self.instances.push(instance);
    }

    fn unbind(&mut self, manager: &ExtWorkspaceManagerV1) {
        self.instances
            .retain(|instance| instance.manager != *manager);
    }

    fn forget_handle(&mut self, id: &wayland_server::backend::ObjectId) {
        for instance in &mut self.instances {
            instance
                .groups
                .retain(|_, group| Resource::id(group) != *id);
            instance
                .workspaces
                .retain(|_, workspace| Resource::id(workspace) != *id);
        }
    }
}

type Model<'a> = (&'a [WorkspaceGroup], &'a [WorkspaceInfo]);

impl Instance {
    /// The protocol's order: new groups, then new and changed workspaces with
    /// their group membership, then removed workspaces before removed groups,
    /// and one `done` for all of it.
    fn send_diff<D: ExtWorkspaceDispatch>(
        &mut self,
        display: &DisplayHandle,
        previous: Model<'_>,
        next: Model<'_>,
    ) {
        let Some(client) = self.manager.client() else {
            return;
        };
        let version = self.manager.version();

        for group in next.0 {
            if self.groups.contains_key(&group.key) {
                continue;
            }
            let Ok(handle) = client
                .create_resource::<ExtWorkspaceGroupHandleV1, _, D>(display, version, group.key)
            else {
                continue;
            };
            self.manager.workspace_group(&handle);
            handle.capabilities(GroupCapabilities::empty());
            for output in group.output.client_outputs(&client) {
                handle.output_enter(&output);
            }
            self.groups.insert(group.key, handle);
        }

        for workspace in next.1 {
            let before = previous.1.iter().find(|old| old.key == workspace.key);
            let handle = match self.workspaces.get(&workspace.key) {
                Some(handle) => handle.clone(),
                None => {
                    let data = WorkspaceData {
                        key: workspace.key,
                        manager: self.manager.clone(),
                    };
                    let Ok(handle) = client
                        .create_resource::<ExtWorkspaceHandleV1, _, D>(display, version, data)
                    else {
                        continue;
                    };
                    self.manager.workspace(&handle);
                    handle.id(workspace.id.clone());
                    handle.capabilities(WorkspaceCapabilities::Activate);
                    self.workspaces.insert(workspace.key, handle.clone());
                    handle
                }
            };
            let fresh = before.is_none();
            if fresh || before.is_some_and(|old| old.name != workspace.name) {
                handle.name(workspace.name.clone());
            }
            if fresh || before.is_some_and(|old| old.coordinates != workspace.coordinates) {
                handle.coordinates(coordinates(&workspace.coordinates));
            }
            if fresh || before.is_some_and(|old| old.active != workspace.active) {
                handle.state(if workspace.active {
                    State::Active
                } else {
                    State::empty()
                });
            }
            let old_group = before.map(|old| old.group).filter(|_| !fresh);
            if old_group != Some(workspace.group) {
                if let Some(group) = old_group.and_then(|key| self.groups.get(&key)) {
                    group.workspace_leave(&handle);
                }
                if let Some(group) = self.groups.get(&workspace.group) {
                    group.workspace_enter(&handle);
                }
            }
        }

        for old in previous.1 {
            if next.1.iter().any(|workspace| workspace.key == old.key) {
                continue;
            }
            if let Some(handle) = self.workspaces.remove(&old.key) {
                if let Some(group) = self.groups.get(&old.group) {
                    group.workspace_leave(&handle);
                }
                handle.removed();
            }
        }

        for old in previous.0 {
            if next.0.iter().any(|group| group.key == old.key) {
                continue;
            }
            if let Some(handle) = self.groups.remove(&old.key) {
                for output in old.output.client_outputs(&client) {
                    handle.output_leave(&output);
                }
                handle.removed();
            }
        }

        self.manager.done();
    }
}

fn coordinates(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// Everything the compositor's state has to dispatch for this protocol.
pub trait ExtWorkspaceDispatch:
    GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData>
    + Dispatch<ExtWorkspaceManagerV1, ManagerData>
    + Dispatch<ExtWorkspaceGroupHandleV1, u64>
    + Dispatch<ExtWorkspaceHandleV1, WorkspaceData>
    + ExtWorkspaceHandler
    + 'static
{
}

impl<D> ExtWorkspaceDispatch for D where
    D: GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData>
        + Dispatch<ExtWorkspaceManagerV1, ManagerData>
        + Dispatch<ExtWorkspaceGroupHandleV1, u64>
        + Dispatch<ExtWorkspaceHandleV1, WorkspaceData>
        + ExtWorkspaceHandler
        + 'static
{
}

/// Activations a client asked for since its last commit.
#[derive(Debug, Default)]
struct Pending(Mutex<Vec<u64>>);

impl Pending {
    fn push(&self, key: u64) {
        self.0
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(key);
    }

    fn take(&self) -> Vec<u64> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|err| err.into_inner()))
    }
}

#[cfg(test)]
mod tests;

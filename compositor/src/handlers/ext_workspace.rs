use smithay::reexports::wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1,
    ext_workspace_handle_v1::ExtWorkspaceHandleV1, ext_workspace_manager_v1::ExtWorkspaceManagerV1,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use protocols::ext_workspace::{
    ExtWorkspaceGlobalData, ExtWorkspaceHandler, ExtWorkspaceState, ManagerData, WorkspaceData,
    WorkspaceGroup, WorkspaceInfo,
};

use crate::{shell::Shell, state::State, utils::id::WorkspaceId};

impl ExtWorkspaceHandler for State {
    fn ext_workspace_state(&mut self) -> &mut ExtWorkspaceState {
        &mut self.wayland.ext_workspace_state
    }

    fn activate_workspace(&mut self, key: u64) {
        let target = self.shell.monitors_mut().iter_mut().find_map(|monitor| {
            let index = monitor
                .workspaces()
                .iter()
                .position(|workspace| workspace.id().raw() == key)?;
            Some((monitor, index))
        });
        if let Some((monitor, index)) = target
            && monitor.activate(index)
        {
            self.queue_redraw();
        }
    }
}

impl State {
    /// Tells `ext_workspace_v1` clients about the workspaces as they are now.
    /// Cheap when nothing changed: the protocol state compares before sending.
    pub fn refresh_ext_workspaces(&mut self) {
        let monitors = self.shell.monitors();
        let groups = monitors
            .iter()
            .map(|monitor| WorkspaceGroup {
                key: monitor.id().raw(),
                output: monitor.output().clone(),
            })
            .collect();
        let workspaces = monitors
            .iter()
            .flat_map(|monitor| {
                let active = monitor.active().id();
                monitor
                    .workspaces()
                    .iter()
                    .enumerate()
                    .map(move |(index, workspace)| WorkspaceInfo {
                        key: workspace.id().raw(),
                        group: monitor.id().raw(),
                        id: workspace_identifier(workspace.id()),
                        name: (index + 1).to_string(),
                        coordinates: vec![index as u32],
                        active: workspace.id() == active,
                    })
            })
            .collect();
        let display = self.common.display_handle.clone();
        self.wayland
            .ext_workspace_state
            .update::<State>(&display, groups, workspaces);
    }
}

/// The `id` clients see, and pass back to `crownos_screencast_v1` to capture a
/// workspace.
pub fn workspace_identifier(id: WorkspaceId) -> String {
    format!("crownos-workspace-{}", id.raw())
}

fn parse_workspace_identifier(identifier: &str) -> Option<u64> {
    identifier.strip_prefix("crownos-workspace-")?.parse().ok()
}

/// The workspace a client named by its `ext_workspace_handle_v1` id.
pub fn workspace_by_identifier(shell: &Shell, identifier: &str) -> Option<WorkspaceId> {
    let raw = parse_workspace_identifier(identifier)?;
    shell
        .monitors()
        .iter()
        .flat_map(|monitor| monitor.workspaces())
        .map(|workspace| workspace.id())
        .find(|id| id.raw() == raw)
}

delegate_global_dispatch!(State: [ExtWorkspaceManagerV1: ExtWorkspaceGlobalData] => ExtWorkspaceState);
delegate_dispatch!(State: [ExtWorkspaceManagerV1: ManagerData] => ExtWorkspaceState);
delegate_dispatch!(State: [ExtWorkspaceGroupHandleV1: u64] => ExtWorkspaceState);
delegate_dispatch!(State: [ExtWorkspaceHandleV1: WorkspaceData] => ExtWorkspaceState);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_identifiers_round_trip() {
        let id = WorkspaceId::next();
        assert_eq!(
            parse_workspace_identifier(&workspace_identifier(id)),
            Some(id.raw())
        );
        assert_eq!(parse_workspace_identifier("crownos-workspace-x"), None);
        assert_eq!(parse_workspace_identifier("7"), None);
    }
}

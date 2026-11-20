//! Which monitors a workspace change acts on. In `PerMonitor` scope that is
//! the monitor it names; in `Shared` scope it is every shown monitor, which
//! then all hold the same number of workspaces and show the same index.
//!
//! Every change to a monitor's workspace list or its active workspace goes
//! through here, so no caller can move one monitor out of step.

mod linking;
mod swipe;

use crownos_config::schema::WorkspaceScope;

use crate::{
    layout::WorkspaceMode,
    shell::{Location, Shell, monitor::Monitor, tile::Tile, workspace::WorkspaceRef},
    utils::id::{OutputId, WindowId, WorkspaceId},
};

/// The most workspaces a launch variable or a window rule may ask for. Past
/// this a number is a typo, not a workspace anyone means to create.
pub const MAX_WORKSPACES: usize = 32;

/// The monitors a change on `from` acts on. A hidden monitor is never
/// linked: it keeps its own workspaces in either scope.
pub fn linked_outputs(scope: WorkspaceScope, from: OutputId, shown: &[OutputId]) -> Vec<OutputId> {
    match scope {
        WorkspaceScope::Shared if shown.contains(&from) => shown.to_vec(),
        _ => vec![from],
    }
}

/// The workspace that takes in the windows of the removed one at `index`:
/// the one before it, or after it for the first. `None` when it is the only
/// workspace, which stays.
pub fn heir_index(index: usize, count: usize) -> Option<usize> {
    let heir = if index == 0 { 1 } else { index - 1 };
    (index < count && heir < count).then_some(heir)
}

impl Shell {
    /// Switching to `Shared` pads every shown monitor and moves them all to
    /// the focused monitor's workspace. Switching back changes nothing.
    pub fn set_workspace_scope(&mut self, scope: WorkspaceScope) {
        self.scope = scope;
        self.link_workspaces();
    }

    /// The monitors a workspace change on `from` acts on, `from` first.
    pub fn linked(&self, from: OutputId) -> Vec<OutputId> {
        let shown: Vec<OutputId> = self.shown_monitors().map(Monitor::id).collect();
        let mut linked = linked_outputs(self.scope, from, &shown);
        if let Some(position) = linked.iter().position(|id| *id == from) {
            linked[..=position].rotate_right(1);
        }
        linked
    }

    fn each_linked_monitor<T>(
        &mut self,
        from: OutputId,
        mut act: impl FnMut(&mut Monitor) -> T,
    ) -> Vec<T> {
        self.linked(from)
            .into_iter()
            .filter_map(|id| self.monitor_by_id_mut(id).map(&mut act))
            .collect()
    }

    /// Switches the focused monitor, and those linked to it, to `target`.
    pub fn switch_workspace(&mut self, target: WorkspaceRef) -> bool {
        let Some((output, index)) = self
            .focused_monitor()
            .and_then(|monitor| Some((monitor.id(), monitor.resolve(target)?)))
        else {
            return false;
        };
        self.activate_index(output, index)
    }

    /// Shows the workspace at `index` on `output` and the monitors linked to
    /// it. Returns whether any of them moved.
    pub fn activate_index(&mut self, output: OutputId, index: usize) -> bool {
        self.each_linked_monitor(output, |monitor| monitor.activate(index))
            .into_iter()
            .fold(false, |moved, changed| moved | changed)
    }

    /// Switches the window's monitor to the workspace it lives on.
    pub fn switch_to_workspace_of(&mut self, id: WindowId) -> bool {
        let Some((output, index)) = self.location(id).and_then(|location| {
            let monitor = self.monitor_by_id(location.output)?;
            Some((location.output, monitor.index_of(location.workspace)?))
        }) else {
            return false;
        };
        self.activate_index(output, index)
    }

    /// Appends a workspace to `output` and the monitors linked to it, and
    /// returns its index on `output`.
    pub fn create_workspace(&mut self, output: OutputId) -> Option<usize> {
        self.each_linked_monitor(output, Monitor::add_workspace)
            .first()
            .copied()
    }

    /// The workspace at `index` on `output`, appending empty ones to it and
    /// to the monitors linked to it until it exists.
    pub fn ensure_workspace(&mut self, output: OutputId, index: usize) -> Option<WorkspaceId> {
        self.each_linked_monitor(output, |monitor| monitor.ensure_workspace(index))
            .first()
            .copied()
    }

    /// Removes the workspace and the one at the same index on each linked
    /// monitor. Each monitor's windows go to its own heir. The last workspace
    /// on a monitor stays.
    pub fn remove_workspace(&mut self, output: OutputId, workspace: WorkspaceId) -> bool {
        let Some(index) = self
            .monitor_by_id(output)
            .and_then(|monitor| monitor.index_of(workspace))
        else {
            return false;
        };
        self.linked(output)
            .into_iter()
            .map(|linked| self.remove_workspace_at(linked, index))
            .fold(false, |removed, this| removed | this)
    }

    fn remove_workspace_at(&mut self, output: OutputId, index: usize) -> bool {
        let Some(monitor) = self.monitor_by_id(output) else {
            return false;
        };
        let workspaces = monitor.workspaces();
        let Some(heir_index) = heir_index(index, workspaces.len()) else {
            return false;
        };
        let removed = workspaces[index].id();
        let heir = workspaces[heir_index].id();
        let was_active = monitor.active_index() == index;
        let orphans: Vec<WindowId> = workspaces[index].tiles().iter().map(Tile::id).collect();

        let heir_location = Location {
            output,
            workspace: heir,
        };
        for id in orphans {
            self.move_tile(id, heir_location);
        }

        let Some(monitor) = self.monitor_by_id_mut(output) else {
            return false;
        };
        monitor.take_workspace(removed);
        if was_active && let Some(heir_index) = monitor.index_of(heir) {
            monitor.activate(heir_index);
        }
        monitor.normalize();
        true
    }

    /// Moves the workspace at `from` to `to` on `output` and the monitors
    /// linked to it.
    pub fn move_workspace(&mut self, output: OutputId, from: usize, to: usize) -> bool {
        self.each_linked_monitor(output, |monitor| monitor.move_workspace(from, to))
            .into_iter()
            .fold(false, |moved, this| moved | this)
    }

    /// Flips the focused monitor's active workspace between tiling and
    /// floating.
    pub fn toggle_workspace_mode(&mut self) -> bool {
        let Some((output, mode)) = self
            .focused_monitor()
            .map(|monitor| (monitor.id(), monitor.active().mode()))
        else {
            return false;
        };
        self.set_workspace_mode_on(output, mode.toggled())
    }

    pub fn set_workspace_mode(&mut self, mode: WorkspaceMode) -> bool {
        self.focused
            .is_some_and(|output| self.set_workspace_mode_on(output, mode))
    }

    /// Sets the mode of `output`'s active workspace, and of the workspace at
    /// the same index on each linked monitor.
    pub fn set_workspace_mode_on(&mut self, output: OutputId, mode: WorkspaceMode) -> bool {
        let Some(index) = self.monitor_by_id(output).map(Monitor::active_index) else {
            return false;
        };
        self.each_linked_monitor(output, |monitor| {
            monitor
                .workspaces_mut()
                .get_mut(index)
                .is_some_and(|workspace| workspace.set_mode(mode))
        })
        .into_iter()
        .fold(false, |changed, this| changed | this)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs(count: u64) -> Vec<OutputId> {
        (1..=count).map(OutputId::from_raw).collect()
    }

    #[test]
    fn per_monitor_scope_links_a_monitor_to_itself_only() {
        let shown = outputs(3);
        assert_eq!(
            linked_outputs(WorkspaceScope::PerMonitor, shown[1], &shown),
            [shown[1]]
        );
    }

    #[test]
    fn shared_scope_links_every_shown_monitor() {
        let shown = outputs(3);
        assert_eq!(
            linked_outputs(WorkspaceScope::Shared, shown[2], &shown),
            shown
        );
    }

    #[test]
    fn a_hidden_monitor_keeps_its_own_workspaces_in_shared_scope() {
        let shown = outputs(2);
        let hidden = OutputId::from_raw(9);
        assert_eq!(
            linked_outputs(WorkspaceScope::Shared, hidden, &shown),
            [hidden]
        );
    }

    #[test]
    fn a_removed_workspace_goes_to_the_one_before_it_or_after_the_first() {
        assert_eq!(heir_index(2, 3), Some(1));
        assert_eq!(heir_index(0, 3), Some(1));
        assert_eq!(heir_index(0, 1), None);
        assert_eq!(heir_index(3, 3), None);
    }
}

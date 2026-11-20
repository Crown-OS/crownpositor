//! Moving a window or a whole workspace to another monitor.
//!
//! In `PerMonitor` scope a window goes to the target's active workspace and a
//! workspace moves whole. In `Shared` scope the workspaces are one list across
//! the monitors, so a window keeps its index and a workspace merges into the
//! target's workspace at the same index.

use crownos_config::schema::WorkspaceScope;

use crate::{
    layout::Direction,
    shell::{Location, Shell, tile::Tile},
    utils::id::{OutputId, WindowId},
};

impl Shell {
    /// Moves a window to `target`: its active workspace in `PerMonitor`
    /// scope, the workspace at the window's own index in `Shared` scope.
    /// A window that held the keyboard keeps it, on its new monitor.
    /// Refused for a hidden or unknown monitor.
    pub fn move_window_to_output(&mut self, id: WindowId, target: OutputId) -> bool {
        let held_keyboard = self.focused_window_id() == Some(id);
        let moved = self.place_on_output(id, target);
        if moved && held_keyboard {
            self.focus_window(id);
        }
        moved
    }

    fn place_on_output(&mut self, id: WindowId, target: OutputId) -> bool {
        let Some(location) = self.location(id).filter(|at| at.output != target) else {
            return false;
        };
        let Some(target_monitor) = self
            .monitor_by_id(target)
            .filter(|monitor| !monitor.is_hidden())
        else {
            return false;
        };
        let index = match self.scope {
            WorkspaceScope::PerMonitor => Some(target_monitor.active_index()),
            WorkspaceScope::Shared => self
                .monitor_by_id(location.output)
                .and_then(|monitor| monitor.index_of(location.workspace)),
        };
        let Some(workspace) = index.and_then(|index| self.ensure_workspace(target, index)) else {
            return false;
        };
        self.move_tile(
            id,
            Location {
                output: target,
                workspace,
            },
        )
    }

    /// Moves the keyboard window onto the monitor next to its own, and
    /// returns that monitor.
    pub fn move_focused_to_output(&mut self, dir: Direction) -> Option<OutputId> {
        let id = self.focused_window_id()?;
        let from = self.location(id)?.output;
        let target = self.monitor_in_direction(from, dir)?;
        if !self.move_window_to_output(id, target) {
            return None;
        }
        self.normalize_all();
        Some(target)
    }

    /// Moves the focused monitor's active workspace to the monitor next to
    /// it, and returns that monitor. In `PerMonitor` scope the workspace moves
    /// whole and becomes active there; in `Shared` scope its windows merge
    /// into the neighbour's workspace at the same index.
    pub fn move_workspace_to_output(&mut self, dir: Direction) -> Option<OutputId> {
        let source = self.focused?;
        let target = self.monitor_in_direction(source, dir)?;
        let moved = match self.scope {
            WorkspaceScope::PerMonitor => self.hand_workspace_over(source, target),
            WorkspaceScope::Shared => self.merge_active_workspace(source, target),
        };
        self.normalize_all();
        moved.then_some(target)
    }

    fn hand_workspace_over(&mut self, source: OutputId, target: OutputId) -> bool {
        if self.monitor_by_id(target).is_none() {
            return false;
        }
        let Some(source_monitor) = self.monitor_by_id_mut(source) else {
            return false;
        };
        let source_output = source_monitor.output().clone();
        let moving = source_monitor.active().id();
        let Some(workspace) = source_monitor.take_workspace(moving) else {
            return false;
        };
        source_monitor.normalize();

        self.adopt(workspace, target, Some(source_output));
        let Some(index) = self
            .monitor_by_id(target)
            .and_then(|monitor| monitor.index_of(moving))
        else {
            return false;
        };
        self.activate_index(target, index);
        true
    }

    fn merge_active_workspace(&mut self, source: OutputId, target: OutputId) -> bool {
        let Some((index, windows, focus)) = self.monitor_by_id(source).map(|monitor| {
            let windows: Vec<WindowId> = monitor.active().tiles().iter().map(Tile::id).collect();
            (monitor.active_index(), windows, monitor.active().focus())
        }) else {
            return false;
        };
        let Some(workspace) = self.ensure_workspace(target, index) else {
            return false;
        };
        let destination = Location {
            output: target,
            workspace,
        };
        for id in windows {
            self.move_tile(id, destination);
        }
        if let Some(focus) = focus {
            self.focus_window(focus);
        }
        true
    }
}

//! Where an unplugged monitor's windows go, and how they come back when it
//! is plugged in again.
//!
//! In `Shared` scope the workspaces are one list across the monitors, so each
//! window moves to the survivor's workspace at its own index. Otherwise the
//! monitor's non-empty workspaces are appended to the survivor whole.

use crownos_config::schema::WorkspaceScope;
use smithay::desktop::space::SpaceElement;

use crate::{
    shell::{
        Location, Shell,
        monitor::{ConnectorId, Monitor},
        tile::Tile,
        workspace::Workspace,
    },
    utils::id::{OutputId, WindowId, WorkspaceId},
};

/// What was rescued from an unplugged monitor, kept until it comes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// Whole workspaces appended to the survivor, and the one that was on
    /// screen when the monitor went.
    Workspaces {
        workspaces: Vec<WorkspaceId>,
        active: Option<WorkspaceId>,
    },
    /// Each window, by the index of the workspace it was on, merged into the
    /// survivor's workspace at that index.
    Merged(Vec<(usize, WindowId)>),
}

/// How an unplugged monitor's windows are rescued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rescue {
    AppendWorkspaces,
    MergeByIndex,
}

impl Rescue {
    /// Merging needs a survivor that shares the workspace list, which only a
    /// shown monitor in `Shared` scope does.
    pub fn choose(scope: WorkspaceScope, departing_is_shown: bool) -> Self {
        match scope {
            WorkspaceScope::Shared if departing_is_shown => Self::MergeByIndex,
            _ => Self::AppendWorkspaces,
        }
    }
}

/// One workspace of the monitor that is going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departing {
    pub id: WorkspaceId,
    pub windows: Vec<WindowId>,
    pub active: bool,
}

/// What to rescue from the departing workspaces, in list order.
pub fn plan_unplug(rescue: Rescue, departing: &[Departing]) -> Migration {
    match rescue {
        Rescue::MergeByIndex => Migration::Merged(
            departing
                .iter()
                .enumerate()
                .flat_map(|(index, workspace)| workspace.windows.iter().map(move |id| (index, *id)))
                .collect(),
        ),
        Rescue::AppendWorkspaces => {
            let kept = departing
                .iter()
                .filter(|workspace| !workspace.windows.is_empty());
            Migration::Workspaces {
                workspaces: kept.clone().map(|workspace| workspace.id).collect(),
                active: kept
                    .clone()
                    .find(|workspace| workspace.active)
                    .map(|workspace| workspace.id),
            }
        }
    }
}

/// How many workspaces the survivor needs before merged windows can land at
/// their indices.
pub fn merged_length(windows: &[(usize, WindowId)]) -> usize {
    windows
        .iter()
        .map(|(index, _)| index + 1)
        .max()
        .unwrap_or(0)
}

/// The merged windows that are still at the index they were merged into,
/// which are the ones a returning monitor takes back. `index_now` is where a
/// window is now on a monitor other than the returning one.
pub fn still_in_place(
    windows: &[(usize, WindowId)],
    index_now: impl Fn(WindowId) -> Option<usize>,
) -> Vec<(usize, WindowId)> {
    windows
        .iter()
        .copied()
        .filter(|(index, id)| index_now(*id) == Some(*index))
        .collect()
}

impl Shell {
    /// The monitor that takes in a departing monitor's windows: the focused
    /// one, else the primary among the others.
    pub(super) fn survivor_for(&self, departing: OutputId) -> Option<OutputId> {
        self.focused
            .filter(|focused| *focused != departing)
            .or_else(|| self.primary_besides(departing).map(Monitor::id))
    }

    /// Takes the monitor out of the list, its windows going to `survivor`,
    /// or to the headless stash when there is none.
    pub(super) fn dismantle(
        &mut self,
        departing: OutputId,
        survivor: Option<OutputId>,
    ) -> Option<Monitor> {
        let monitor = self.monitor_by_id(departing)?;
        let rescue = match survivor {
            Some(_) => Rescue::choose(self.scope, !monitor.is_hidden()),
            None => Rescue::AppendWorkspaces,
        };
        let active = monitor.active().id();
        let summary: Vec<Departing> = monitor
            .workspaces()
            .iter()
            .map(|workspace| Departing {
                id: workspace.id(),
                windows: workspace.tiles().iter().map(Tile::id).collect(),
                active: workspace.id() == active,
            })
            .collect();
        let connector = monitor.config().connector.clone();
        let migration = plan_unplug(rescue, &summary);

        if let (Migration::Merged(windows), Some(target)) = (&migration, survivor) {
            self.merge_windows(windows, target);
        }

        let index = self
            .monitors
            .iter()
            .position(|monitor| monitor.id() == departing)?;
        let mut monitor = self.monitors.remove(index);
        let mut orphans = monitor.drain_workspaces();
        orphans.retain(|workspace| !workspace.is_empty());
        for tile in orphans.iter().flat_map(Workspace::tiles) {
            tile.window().output_leave(monitor.output());
        }

        match survivor {
            Some(target) => {
                self.migrated.insert(connector, migration);
                for workspace in orphans {
                    self.adopt(workspace, target);
                }
            }
            None => self.headless.extend(orphans),
        }
        Some(monitor)
    }

    fn merge_windows(&mut self, windows: &[(usize, WindowId)], target: OutputId) {
        if let Some(monitor) = self.monitor_by_id_mut(target) {
            monitor.pad_to(merged_length(windows));
        }
        for &(index, id) in windows {
            let Some(workspace) = self
                .monitor_by_id(target)
                .and_then(|monitor| monitor.workspaces().get(index))
                .map(Workspace::id)
            else {
                continue;
            };
            self.move_tile(
                id,
                Location {
                    output: target,
                    workspace,
                },
            );
        }
    }

    /// Gives a returning monitor back what was rescued from it, whichever
    /// scope is active now.
    pub(super) fn reclaim_migrated(&mut self, connector: &ConnectorId, target: OutputId) {
        match self.migrated.remove(connector) {
            Some(Migration::Workspaces { workspaces, active }) => {
                self.reclaim_workspaces(&workspaces, target);
                if let Some(monitor) = self.monitor_by_id_mut(target)
                    && let Some(index) = active.and_then(|active| monitor.index_of(active))
                {
                    monitor.activate(index);
                }
            }
            Some(Migration::Merged(windows)) => self.reclaim_merged(&windows, target),
            None => {}
        }
    }

    fn reclaim_workspaces(&mut self, ids: &[WorkspaceId], target: OutputId) {
        let mut reclaimed = Vec::new();
        for id in ids {
            let Some(source) = self
                .monitors
                .iter_mut()
                .find(|monitor| monitor.id() != target && monitor.index_of(*id).is_some())
            else {
                continue;
            };
            let Some(workspace) = source.take_workspace(*id) else {
                continue;
            };
            for tile in workspace.tiles() {
                tile.window().output_leave(source.output());
            }
            reclaimed.push(workspace);
        }
        self.adopt_all(reclaimed, target);
    }

    fn reclaim_merged(&mut self, windows: &[(usize, WindowId)], target: OutputId) {
        let returning = still_in_place(windows, |id| {
            let location = self.location(id).filter(|at| at.output != target)?;
            self.monitor_by_id(location.output)?
                .index_of(location.workspace)
        });
        for (index, id) in returning {
            let Some(workspace) = self.ensure_workspace(target, index) else {
                continue;
            };
            self.move_tile(
                id,
                Location {
                    output: target,
                    workspace,
                },
            );
        }
    }

    /// Hands the windows parked while no monitor was shown to the first one
    /// that is.
    pub(super) fn adopt_headless(&mut self, target: OutputId) {
        let stashed = std::mem::take(&mut self.headless);
        self.adopt_all(stashed, target);
    }

    /// Adopts a list of workspaces in place of the empty one a new monitor
    /// starts with, so they keep the indices they had.
    fn adopt_all(&mut self, workspaces: Vec<Workspace>, target: OutputId) {
        if workspaces.is_empty() {
            return;
        }
        if let Some(monitor) = self.monitor_by_id_mut(target)
            && let Some(fresh) = monitor.lone_fresh_workspace()
        {
            monitor.take_workspace(fresh);
        }
        for workspace in workspaces {
            self.adopt(workspace, target);
        }
        if let Some(monitor) = self.monitor_by_id_mut(target) {
            monitor.normalize();
        }
    }

    /// Moves a rescued workspace onto the end of a monitor's list.
    pub(super) fn adopt(&mut self, workspace: Workspace, target: OutputId) {
        let ids: Vec<WindowId> = workspace.tiles().iter().map(Tile::id).collect();
        let workspace_id = workspace.id();

        let Some(monitor) = self.monitor_by_id_mut(target) else {
            self.headless.push(workspace);
            return;
        };

        let output = monitor.output().clone();
        monitor.push_workspace(workspace);
        self.readvertise_output(&output);

        for id in ids {
            self.window_to_location.insert(
                id,
                Location {
                    output: target,
                    workspace: workspace_id,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(windows: &[WindowId], active: bool) -> Departing {
        Departing {
            id: WorkspaceId::next(),
            windows: windows.to_vec(),
            active,
        }
    }

    #[test]
    fn shared_scope_merges_each_window_by_its_index() {
        let (first, second, third) = (WindowId::next(), WindowId::next(), WindowId::next());
        let departing = [
            workspace(&[], false),
            workspace(&[first, second], true),
            workspace(&[third], false),
        ];
        assert_eq!(
            plan_unplug(Rescue::MergeByIndex, &departing),
            Migration::Merged(vec![(1, first), (1, second), (2, third)])
        );
    }

    #[test]
    fn per_monitor_scope_appends_only_workspaces_with_windows() {
        let window = WindowId::next();
        let departing = [workspace(&[], true), workspace(&[window], false)];
        assert_eq!(
            plan_unplug(Rescue::AppendWorkspaces, &departing),
            Migration::Workspaces {
                workspaces: vec![departing[1].id],
                active: None,
            }
        );
    }

    #[test]
    fn the_workspace_on_screen_is_remembered_when_it_had_windows() {
        let window = WindowId::next();
        let departing = [workspace(&[], false), workspace(&[window], true)];
        assert_eq!(
            plan_unplug(Rescue::AppendWorkspaces, &departing),
            Migration::Workspaces {
                workspaces: vec![departing[1].id],
                active: Some(departing[1].id),
            }
        );
    }

    #[test]
    fn only_a_shown_monitor_in_shared_scope_merges() {
        assert_eq!(
            Rescue::choose(WorkspaceScope::Shared, true),
            Rescue::MergeByIndex
        );
        assert_eq!(
            Rescue::choose(WorkspaceScope::Shared, false),
            Rescue::AppendWorkspaces
        );
        assert_eq!(
            Rescue::choose(WorkspaceScope::PerMonitor, true),
            Rescue::AppendWorkspaces
        );
    }

    #[test]
    fn the_survivor_is_padded_to_the_highest_merged_index() {
        let window = WindowId::next();
        assert_eq!(merged_length(&[(0, window), (3, window)]), 4);
        assert_eq!(merged_length(&[]), 0);
    }

    #[test]
    fn a_window_moved_since_the_merge_stays_where_the_user_put_it() {
        let (stayed, moved, closed) = (WindowId::next(), WindowId::next(), WindowId::next());
        let merged = [(1, stayed), (2, moved), (2, closed)];
        let index_now = |id| match id {
            id if id == stayed => Some(1),
            id if id == moved => Some(0),
            _ => None,
        };
        assert_eq!(still_in_place(&merged, index_now), [(1, stayed)]);
    }
}

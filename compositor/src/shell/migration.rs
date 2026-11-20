//! Where an unplugged monitor's windows go, and how they come back when it
//! is plugged in again.
//!
//! In `Shared` scope the workspaces are one list across the monitors, so each
//! window moves to the survivor's workspace at its own index. Otherwise the
//! monitor's non-empty workspaces are appended to the survivor whole.

use std::collections::HashSet;

use crownos_config::schema::WorkspaceScope;
use smithay::desktop::space::SpaceElement;

use crate::{
    shell::{
        Location, Shell,
        arrangement::nearest_by_centre,
        monitor::{Monitor, OutputConfig},
        stash::kept_for_stash,
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

impl Migration {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Workspaces { workspaces, .. } => workspaces.is_empty(),
            Self::Merged(windows) => windows.is_empty(),
        }
    }
}

/// Which monitor a migration belongs to. The EDID identity follows a monitor
/// to another port; the connector tells twins with one identity apart.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MigrationKey {
    pub edid_identity: String,
    pub connector: String,
}

impl MigrationKey {
    pub fn of(config: &OutputConfig) -> Self {
        Self {
            edid_identity: config.edid_identity.clone(),
            connector: config.name.clone(),
        }
    }
}

/// The record a returning monitor takes back: the one filed under its EDID
/// identity and connector, else the only one filed under its EDID identity,
/// which is the same monitor plugged into another port.
pub fn claimed_key<'a>(
    keys: impl Iterator<Item = &'a MigrationKey> + Clone,
    returning: &MigrationKey,
) -> Option<&'a MigrationKey> {
    keys.clone().find(|key| *key == returning).or_else(|| {
        let mut same_monitor = keys.filter(|key| key.edid_identity == returning.edid_identity);
        match (same_monitor.next(), same_monitor.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
    })
}

/// How an unplugged monitor's windows are rescued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rescue {
    AppendWorkspaces,
    MergeByIndex,
}

impl Rescue {
    /// Merging keeps the workspace list shared, which only a shown monitor in
    /// `Shared` scope does.
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

/// The windows and workspaces that already belong to the record of a monitor
/// that left earlier. Each belongs to one record only, so a monitor that
/// leaves after them carries them on without claiming them.
#[derive(Debug, Default)]
pub struct Owned {
    windows: HashSet<WindowId>,
    workspaces: HashSet<WorkspaceId>,
}

impl Owned {
    pub fn by<'a>(records: impl Iterator<Item = &'a Migration>) -> Self {
        let mut owned = Self::default();
        for record in records {
            match record {
                Migration::Workspaces { workspaces, .. } => {
                    owned.workspaces.extend(workspaces.iter().copied());
                }
                Migration::Merged(windows) => {
                    owned.windows.extend(windows.iter().map(|(_, id)| *id));
                }
            }
        }
        owned
    }

    fn holds(&self, workspace: WorkspaceId, window: WindowId) -> bool {
        self.workspaces.contains(&workspace) || self.windows.contains(&window)
    }
}

/// Every window of the departing workspaces, with the index it was at.
pub fn indexed_windows(departing: &[Departing]) -> Vec<(usize, WindowId)> {
    departing
        .iter()
        .enumerate()
        .flat_map(|(index, workspace)| workspace.windows.iter().map(move |id| (index, *id)))
        .collect()
}

/// What the departing monitor's own record holds, in list order. What an
/// earlier record already owns moves with the rest but stays with that record.
pub fn plan_unplug(rescue: Rescue, departing: &[Departing], owned: &Owned) -> Migration {
    match rescue {
        Rescue::MergeByIndex => Migration::Merged(
            departing
                .iter()
                .enumerate()
                .flat_map(|(index, workspace)| {
                    workspace
                        .windows
                        .iter()
                        .filter(|id| !owned.holds(workspace.id, **id))
                        .map(move |id| (index, *id))
                })
                .collect(),
        ),
        Rescue::AppendWorkspaces => {
            let kept = departing.iter().filter(|workspace| {
                !workspace.windows.is_empty() && !owned.workspaces.contains(&workspace.id)
            });
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
    /// one, else the shown monitor nearest to it, else the primary.
    pub(super) fn survivor_for(&self, departing: OutputId) -> Option<OutputId> {
        let focused = self.focused.filter(|focused| *focused != departing);
        let nearest = || {
            let from = self.monitor_by_id(departing)?.geometry();
            nearest_by_centre(
                from,
                self.shown_monitors()
                    .filter(|monitor| monitor.id() != departing)
                    .map(|monitor| (monitor.id(), monitor.geometry())),
            )
        };
        focused
            .or_else(nearest)
            .or_else(|| self.primary_besides(departing).map(Monitor::id))
    }

    /// Takes the monitor out of the list, its windows going to `survivor`,
    /// or to the stash when there is none. Either way they are recorded
    /// against the monitor, so they come back with it.
    pub(super) fn dismantle(
        &mut self,
        departing: OutputId,
        survivor: Option<OutputId>,
    ) -> Option<Monitor> {
        let monitor = self.monitor_by_id(departing)?;
        let rescue = Rescue::choose(self.scope, !monitor.is_hidden());
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
        let key = MigrationKey::of(monitor.config());
        let record = plan_unplug(rescue, &summary, &Owned::by(self.migrated.values()));

        if let (Rescue::MergeByIndex, Some(target)) = (rescue, survivor) {
            self.merge_windows(&indexed_windows(&summary), target);
        }

        let index = self
            .monitors
            .iter()
            .position(|monitor| monitor.id() == departing)?;
        let mut monitor = self.monitors.remove(index);
        let orphans = monitor.drain_workspaces();
        for tile in orphans.iter().flat_map(Workspace::tiles) {
            tile.window().output_leave(monitor.output());
        }

        match survivor {
            Some(target) => {
                for workspace in orphans
                    .into_iter()
                    .filter(|workspace| !workspace.is_empty())
                {
                    self.adopt(workspace, target);
                }
            }
            None => self.stash(kept_for_stash(orphans, Workspace::is_empty)),
        }
        if !record.is_empty() {
            self.migrated.insert(key, record);
        }
        Some(monitor)
    }

    fn merge_windows(&mut self, windows: &[(usize, WindowId)], target: OutputId) {
        let modes = self.linked_modes();
        if let Some(monitor) = self.monitor_by_id_mut(target) {
            monitor.pad_to(merged_length(windows), &modes);
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
    pub(super) fn reclaim_migrated(&mut self, returning: &MigrationKey, target: OutputId) {
        let Some(key) = claimed_key(self.migrated.keys(), returning).cloned() else {
            return;
        };
        match self.migrated.remove(&key) {
            Some(Migration::Workspaces { workspaces, active }) => {
                self.reclaim_workspaces(&workspaces, target);
                if let Some(index) = active.and_then(|active| {
                    self.monitor_by_id(target)
                        .and_then(|monitor| monitor.index_of(active))
                }) && self.switches_alone(target)
                {
                    self.activate_index(target, index);
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
            let survivor_mode = self
                .location(id)
                .and_then(|at| self.workspace(at))
                .map(Workspace::mode);
            let Some(workspace) = self.ensure_workspace(target, index) else {
                continue;
            };
            let destination = Location {
                output: target,
                workspace,
            };
            if let (Some(mode), Some(empty)) = (
                survivor_mode,
                self.workspace_mut(destination)
                    .filter(|workspace| workspace.is_empty()),
            ) {
                empty.set_mode(mode);
            }
            self.move_tile(id, destination);
        }
    }

    /// Adopts a list of workspaces in place of the empty one a new monitor
    /// starts with, so they keep the indices they had.
    pub(super) fn adopt_all(&mut self, workspaces: Vec<Workspace>, target: OutputId) {
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
            self.stash(vec![workspace]);
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
            plan_unplug(Rescue::MergeByIndex, &departing, &Owned::default()),
            Migration::Merged(vec![(1, first), (1, second), (2, third)])
        );
    }

    #[test]
    fn per_monitor_scope_appends_only_workspaces_with_windows() {
        let window = WindowId::next();
        let departing = [workspace(&[], true), workspace(&[window], false)];
        assert_eq!(
            plan_unplug(Rescue::AppendWorkspaces, &departing, &Owned::default()),
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
            plan_unplug(Rescue::AppendWorkspaces, &departing, &Owned::default()),
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

    #[test]
    fn a_window_an_earlier_record_owns_moves_on_but_stays_with_that_record() {
        let (first_away, own) = (WindowId::next(), WindowId::next());
        let earlier = Migration::Merged(vec![(1, first_away)]);
        let departing = [workspace(&[], false), workspace(&[first_away, own], true)];

        let owned = Owned::by([&earlier].into_iter());
        assert_eq!(
            plan_unplug(Rescue::MergeByIndex, &departing, &owned),
            Migration::Merged(vec![(1, own)])
        );
        assert_eq!(indexed_windows(&departing), [(1, first_away), (1, own)]);
    }

    #[test]
    fn a_workspace_an_earlier_record_owns_is_not_appended_to_a_second_one() {
        let (first_away, own) = (WindowId::next(), WindowId::next());
        let departing = [workspace(&[own], true), workspace(&[first_away], false)];
        let earlier = Migration::Workspaces {
            workspaces: vec![departing[1].id],
            active: Some(departing[1].id),
        };

        let owned = Owned::by([&earlier].into_iter());
        assert_eq!(
            plan_unplug(Rescue::AppendWorkspaces, &departing, &owned),
            Migration::Workspaces {
                workspaces: vec![departing[0].id],
                active: Some(departing[0].id),
            }
        );
    }

    fn key(edid_identity: &str, connector: &str) -> MigrationKey {
        MigrationKey {
            edid_identity: edid_identity.to_owned(),
            connector: connector.to_owned(),
        }
    }

    #[test]
    fn twins_take_back_only_what_left_through_their_own_connector() {
        let keys = [key("Dell A1", "DP-1"), key("Dell A1", "DP-2")];
        assert_eq!(
            claimed_key(keys.iter(), &key("Dell A1", "DP-1")),
            Some(&keys[0])
        );
        assert_eq!(claimed_key(keys.iter(), &key("Dell A1", "DP-3")), None);
    }

    #[test]
    fn a_monitor_plugged_into_another_port_takes_back_its_own_record() {
        let keys = [key("Dell A1", "DP-1"), key("eDP-1", "eDP-1")];
        assert_eq!(
            claimed_key(keys.iter(), &key("Dell A1", "DP-4")),
            Some(&keys[0])
        );
        assert_eq!(claimed_key(keys.iter(), &key("HDMI-A-1", "HDMI-A-1")), None);
    }

    #[test]
    fn a_twin_that_left_first_and_returns_alone_still_finds_its_record() {
        let keys = [key("Dell A1", "DP-2"), key("Dell A1", "DP-1")];
        assert_eq!(
            claimed_key(keys.iter(), &key("Dell A1", "DP-1")),
            Some(&keys[1])
        );
    }
}

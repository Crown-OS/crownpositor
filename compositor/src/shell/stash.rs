//! Where workspaces wait while no shown monitor is left to take them in.
//!
//! A stashed workspace and its windows name `STASH` as their output, so the
//! window index stays valid: a window closed while it waits is found and
//! removed like any other.

use crate::{
    shell::{Location, Shell, tile::Tile, workspace::Workspace},
    utils::id::{OutputId, WorkspaceId},
};

/// The output of every stashed workspace. Real output ids start at 1.
pub const STASH: OutputId = OutputId::from_raw(0);

/// The workspaces worth keeping from a monitor that goes to the stash: all
/// but the empty ones at the end, so every window keeps its index.
pub fn kept_for_stash<T>(mut workspaces: Vec<T>, is_empty: impl Fn(&T) -> bool) -> Vec<T> {
    let kept = workspaces
        .iter()
        .rposition(|workspace| !is_empty(workspace))
        .map_or(0, |last| last + 1);
    workspaces.truncate(kept);
    workspaces
}

impl Shell {
    /// Parks workspaces until a shown monitor comes back.
    pub(super) fn stash(&mut self, workspaces: Vec<Workspace>) {
        for mut workspace in workspaces {
            workspace.set_output(STASH);
            let location = Location {
                output: STASH,
                workspace: workspace.id(),
            };
            for id in workspace.tiles().iter().map(Tile::id) {
                self.window_to_location.insert(id, location);
            }
            self.stashed.push(workspace);
        }
    }

    pub(super) fn stashed_workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.stashed.iter().find(|workspace| workspace.id() == id)
    }

    pub(super) fn stashed_workspace_mut(&mut self, id: WorkspaceId) -> Option<&mut Workspace> {
        self.stashed
            .iter_mut()
            .find(|workspace| workspace.id() == id)
    }

    /// Hands every stashed workspace to the first shown monitor to arrive.
    pub(super) fn adopt_stash(&mut self, target: OutputId) {
        let stashed = std::mem::take(&mut self.stashed);
        self.adopt_all(stashed, target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_empty_workspaces_at_the_end_are_dropped() {
        let workspaces = vec![0, 2, 0, 1, 0, 0];
        assert_eq!(
            kept_for_stash(workspaces, |windows| *windows == 0),
            [0, 2, 0, 1]
        );
        assert!(kept_for_stash(vec![0, 0], |windows| *windows == 0).is_empty());
    }

    #[test]
    fn the_stash_is_no_output_id_a_monitor_can_have() {
        assert_ne!(OutputId::next(), STASH);
    }
}

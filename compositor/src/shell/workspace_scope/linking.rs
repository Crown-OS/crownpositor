//! Keeps the shown monitors in step while the scope is `Shared`.

use crownos_config::schema::WorkspaceScope;

use crate::{
    layout::WorkspaceMode,
    shell::{Shell, monitor::Monitor},
    utils::id::OutputId,
};

/// How many workspaces each linked monitor needs: as many as the longest
/// list, so padding only ever adds.
pub fn linked_length(lengths: impl Iterator<Item = usize>) -> usize {
    lengths.max().unwrap_or(0)
}

impl Shell {
    /// In `Shared` scope, pads every shown monitor with empty workspaces at
    /// the end up to the longest list, and slides each to the focused
    /// monitor's active workspace. Does nothing when they already agree, so
    /// it runs on every refresh.
    pub(in crate::shell) fn link_workspaces(&mut self) {
        if self.scope != WorkspaceScope::Shared {
            return;
        }
        let Some((active, previous)) = self
            .focused_monitor()
            .filter(|monitor| !monitor.is_hidden())
            .map(|monitor| (monitor.active_index(), monitor.previous_index()))
        else {
            return;
        };
        let length = linked_length(
            self.shown_monitors()
                .map(|monitor| monitor.workspaces().len()),
        );
        let modes = self.linked_modes();
        for monitor in self
            .monitors
            .iter_mut()
            .filter(|monitor| !monitor.is_hidden())
        {
            monitor.pad_to(length, &modes);
            monitor.align_to(active, previous);
        }
    }

    /// The modes a workspace created at each index takes so it matches the
    /// focused monitor: in `Shared` scope that monitor's modes, else none.
    pub(in crate::shell) fn linked_modes(&self) -> Vec<WorkspaceMode> {
        match self.scope {
            WorkspaceScope::Shared => self
                .focused_monitor()
                .filter(|monitor| !monitor.is_hidden())
                .map(Monitor::workspace_modes)
                .unwrap_or_default(),
            WorkspaceScope::PerMonitor => Vec::new(),
        }
    }

    /// Gives the empty workspaces of a monitor that has just arrived the
    /// modes of the monitor it links to.
    pub(in crate::shell) fn follow_linked_modes(&mut self, output: OutputId) {
        if self.focused == Some(output) {
            return;
        }
        let modes = self.linked_modes();
        if let Some(monitor) = self
            .monitor_by_id_mut(output)
            .filter(|monitor| !monitor.is_hidden())
        {
            monitor.follow_modes(&modes);
        }
    }

    /// Whether every shown monitor holds the same number of workspaces and
    /// shows the same index, which `Shared` scope promises.
    #[cfg(debug_assertions)]
    pub fn workspaces_are_linked(&self) -> bool {
        let mut shown = self
            .shown_monitors()
            .map(|monitor| (monitor.workspaces().len(), monitor.active_index()));
        let Some(first) = shown.next() else {
            return true;
        };
        shown.all(|other| other == first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_reaches_the_longest_list() {
        assert_eq!(linked_length([1, 4, 2].into_iter()), 4);
        assert_eq!(linked_length(std::iter::empty()), 0);
    }
}

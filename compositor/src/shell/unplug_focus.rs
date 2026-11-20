//! The keyboard after an unplug. The window that had it on the removed
//! monitor keeps it when it lands where the survivor shows it.

use crate::{
    shell::{Location, Shell},
    utils::id::{OutputId, WindowId},
};

/// The window to give the keyboard: the one that held it on the removed
/// monitor, when it landed on the survivor's active workspace.
pub fn keyboard_window_after_unplug(
    held: Option<WindowId>,
    landed: Option<Location>,
    survivor_active: Option<Location>,
) -> Option<WindowId> {
    held.filter(|_| landed.is_some() && landed == survivor_active)
}

impl Shell {
    /// The window that holds the keyboard when it is on `departing`.
    pub(super) fn keyboard_window_on(&self, departing: OutputId) -> Option<WindowId> {
        self.focused_window_id()
            .filter(|_| self.keyboard == Some(departing))
    }

    /// Gives `held` the keyboard again if it landed on the survivor's
    /// active workspace.
    pub(super) fn return_keyboard(&mut self, held: Option<WindowId>, survivor: Option<OutputId>) {
        let survivor_active = survivor
            .and_then(|survivor| self.monitor_by_id(survivor))
            .map(|monitor| Location {
                output: monitor.id(),
                workspace: monitor.active().id(),
            });
        let landed = held.and_then(|window| self.location(window));
        if let Some(window) = keyboard_window_after_unplug(held, landed, survivor_active) {
            self.focus_window(window);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::id::WorkspaceId;

    fn location() -> Location {
        Location {
            output: OutputId::from_raw(7),
            workspace: WorkspaceId::next(),
        }
    }

    #[test]
    fn the_window_keeps_the_keyboard_on_the_survivors_active_workspace() {
        let window = WindowId::next();
        let active = location();
        assert_eq!(
            keyboard_window_after_unplug(Some(window), Some(active), Some(active)),
            Some(window)
        );
    }

    #[test]
    fn a_window_parked_on_an_inactive_workspace_does_not_take_the_keyboard() {
        let window = WindowId::next();
        assert_eq!(
            keyboard_window_after_unplug(Some(window), Some(location()), Some(location())),
            None
        );
        assert_eq!(keyboard_window_after_unplug(Some(window), None, None), None);
        assert_eq!(
            keyboard_window_after_unplug(None, Some(location()), Some(location())),
            None
        );
    }
}

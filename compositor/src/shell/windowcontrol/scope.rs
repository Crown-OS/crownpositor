//! Which windows the Alt+Tab stack offers. In `Shared` scope it is every
//! window on the active workspaces of the linked monitors, most recent first;
//! in `PerMonitor` scope it is the focused monitor's active workspace alone.

use windowcontrol::Direction;

use super::WindowControl;
use crate::{
    shell::{Shell, monitor::Monitor, tile::Tile, workspace::Workspace},
    utils::id::{OutputId, WindowId},
};

/// The workspaces a stack draws its cards from, and their windows in the
/// order the stack lists them.
pub struct SwitcherWindows<'a> {
    pub workspaces: Vec<&'a Workspace>,
    pub recent: Vec<WindowId>,
}

impl SwitcherWindows<'_> {
    pub fn tile(&self, window: WindowId) -> Option<&Tile> {
        self.workspaces
            .iter()
            .find_map(|workspace| workspace.tile(window))
    }
}

impl Shell {
    /// The windows a stack on `owner` offers: those on the active workspaces
    /// of `owner` and the monitors linked to it.
    pub fn switcher_windows(&self, owner: OutputId) -> SwitcherWindows<'_> {
        let workspaces: Vec<&Workspace> = self
            .linked(owner)
            .into_iter()
            .filter_map(|output| self.monitor_by_id(output))
            .map(Monitor::active)
            .collect();
        let recent = match workspaces.as_slice() {
            [only] => only.recent_windows().collect(),
            several => {
                let each: Vec<Vec<WindowId>> = several
                    .iter()
                    .map(|workspace| workspace.recent_windows().collect())
                    .collect();
                self.focus_history
                    .most_recent_first(self.focused_window_id(), &each)
            }
        };
        SwitcherWindows { workspaces, recent }
    }

    /// The stack on `owner` with the windows it offers, borrowed together.
    pub fn with_window_control<T>(
        &mut self,
        owner: OutputId,
        act: impl FnOnce(&mut WindowControl, &Monitor, &SwitcherWindows<'_>) -> T,
    ) -> Option<T> {
        let index = self
            .monitors
            .iter()
            .position(|monitor| monitor.id() == owner)?;
        let mut control = std::mem::take(self.monitors[index].window_control_mut());
        let windows = self.switcher_windows(owner);
        let result = act(&mut control, &self.monitors[index], &windows);
        *self.monitors[index].window_control_mut() = control;
        Some(result)
    }

    /// The window after the one with the keyboard, walking the active
    /// workspaces of the monitors in scope left to right.
    pub fn next_window_in_scope(&self, direction: Direction) -> Option<WindowId> {
        let scope = self.scope_monitors();
        let order: Vec<WindowId> = self
            .shown_monitors()
            .filter(|monitor| scope.contains(&monitor.id()))
            .flat_map(|monitor| monitor.active().tiles().iter().map(Tile::id))
            .collect();
        let current = self
            .focused_window_id()
            .filter(|window| order.contains(window))
            .or_else(|| self.focused_monitor()?.active().focus());
        next_in_cycle(&order, current, direction)
    }
}

/// The window after `current` in `order`, wrapping at either end; the first
/// when `current` is not in it.
fn next_in_cycle(
    order: &[WindowId],
    current: Option<WindowId>,
    direction: Direction,
) -> Option<WindowId> {
    let len = order.len();
    let at = current.and_then(|current| order.iter().position(|window| *window == current));
    let index = match (at, direction) {
        (None, _) => 0,
        (Some(at), Direction::Forward) => (at + 1) % len,
        (Some(at), Direction::Backward) => (at + len - 1) % len,
    };
    order.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows<const COUNT: usize>() -> [WindowId; COUNT] {
        std::array::from_fn(|_| WindowId::next())
    }

    #[test]
    fn the_walk_crosses_from_one_workspace_into_the_next_and_wraps() {
        let [first_left, last_left, first_right] = windows();
        let order = [first_left, last_left, first_right];

        assert_eq!(
            next_in_cycle(&order, Some(last_left), Direction::Forward),
            Some(first_right)
        );
        assert_eq!(
            next_in_cycle(&order, Some(first_right), Direction::Forward),
            Some(first_left)
        );
        assert_eq!(
            next_in_cycle(&order, Some(first_left), Direction::Backward),
            Some(first_right)
        );
    }

    #[test]
    fn the_walk_starts_at_the_first_window_without_a_current_one() {
        let [first, second] = windows();
        let elsewhere = WindowId::next();

        assert_eq!(
            next_in_cycle(&[first, second], None, Direction::Backward),
            Some(first)
        );
        assert_eq!(
            next_in_cycle(&[first, second], Some(elsewhere), Direction::Forward),
            Some(first)
        );
    }

    #[test]
    fn an_empty_walk_goes_nowhere() {
        assert_eq!(next_in_cycle(&[], None, Direction::Forward), None);
        assert_eq!(
            next_in_cycle(&[], Some(WindowId::next()), Direction::Forward),
            None
        );
    }
}

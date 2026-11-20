//! Whether a window that has just mapped takes the keyboard.

use crate::{
    shell::{Location, Shell},
    utils::id::WindowId,
};

/// What a new window is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapFocus {
    /// The keyboard, on the monitor the window mapped on.
    Keyboard,
    /// Only its workspace's focus, so it has the keyboard when the user
    /// switches there. Keys typed at the screen stay where they were.
    Workspace,
    Nothing,
}

impl MapFocus {
    /// A window placed on a workspace nobody sees cannot take the keyboard
    /// from the one the user types at.
    pub fn decide(focus_rule: Option<bool>, on_screen: bool) -> Self {
        match (focus_rule.unwrap_or(true), on_screen) {
            (false, _) => Self::Nothing,
            (true, true) => Self::Keyboard,
            (true, false) => Self::Workspace,
        }
    }
}

impl Shell {
    /// Whether `at` is the active workspace of a shown monitor.
    pub fn is_on_screen(&self, at: Location) -> bool {
        self.active_location_on(at.output) == Some(at)
    }

    /// Gives a window that has just mapped the focus its rule and its
    /// placement allow.
    pub fn focus_mapped(&mut self, id: WindowId, focus_rule: Option<bool>) {
        let Some(location) = self.location(id) else {
            return;
        };
        match MapFocus::decide(focus_rule, self.is_on_screen(location)) {
            MapFocus::Keyboard => {
                self.focus_window(id);
            }
            MapFocus::Workspace => {
                if let Some(workspace) = self.workspace_mut(location) {
                    workspace.focus_window(id);
                }
            }
            MapFocus::Nothing => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_mapped_on_screen_takes_the_keyboard() {
        assert_eq!(MapFocus::decide(None, true), MapFocus::Keyboard);
        assert_eq!(MapFocus::decide(Some(true), true), MapFocus::Keyboard);
    }

    #[test]
    fn a_window_mapped_off_screen_leaves_the_keyboard_where_it_is() {
        assert_eq!(MapFocus::decide(None, false), MapFocus::Workspace);
        assert_eq!(MapFocus::decide(Some(true), false), MapFocus::Workspace);
    }

    #[test]
    fn a_rule_that_refuses_focus_is_honoured_anywhere() {
        assert_eq!(MapFocus::decide(Some(false), true), MapFocus::Nothing);
        assert_eq!(MapFocus::decide(Some(false), false), MapFocus::Nothing);
    }
}

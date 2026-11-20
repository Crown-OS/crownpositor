//! What a key does while the overview owns the keyboard.

use crate::input::shortcuts::Action;

/// Escape closes the overview, and the shortcuts that open, close or toggle it
/// keep their meaning. Every other key is swallowed: the windows behind the
/// overview are pictures of windows, not somewhere to type.
pub(super) fn overview_key(escape: bool, bound: Option<Action>) -> Action {
    match bound {
        _ if escape => Action::CloseWorkspaceView,
        Some(
            action @ (Action::ToggleWorkspaceView
            | Action::CloseWorkspaceView
            | Action::OpenWorkspaceView),
        ) => action,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::shortcuts::WorkspaceRef;

    #[test]
    fn escape_closes_the_overview() {
        assert_eq!(overview_key(true, None), Action::CloseWorkspaceView);
    }

    #[test]
    fn the_toggle_shortcut_closes_an_open_overview_again() {
        assert_eq!(
            overview_key(false, Some(Action::ToggleWorkspaceView)),
            Action::ToggleWorkspaceView
        );
        assert_eq!(
            overview_key(false, Some(Action::CloseWorkspaceView)),
            Action::CloseWorkspaceView
        );
    }

    #[test]
    fn other_shortcuts_and_plain_keys_are_swallowed() {
        assert_eq!(
            overview_key(false, Some(Action::Workspace(WorkspaceRef::Index(1)))),
            Action::None
        );
        assert_eq!(overview_key(false, None), Action::None);
    }
}

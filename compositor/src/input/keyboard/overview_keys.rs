//! What a key does while the overview owns the keyboard.

use smithay::input::keyboard::Keysym;

use crate::input::shortcuts::Action;

/// The Escape keysym closes the overview. With Caps Lock and Escape swapped, that
/// is the key labelled Caps Lock.
pub(super) fn closes_overview(keysyms: &[Keysym]) -> bool {
    keysyms.contains(&Keysym::Escape)
}

/// Lock keys reach the client even while the overview owns the keyboard. The
/// keyboard state toggles the lock before the filter runs, so a swallowed lock
/// key would leave the client with a lock state that is not true.
pub(super) fn is_lock_key(keysyms: &[Keysym]) -> bool {
    keysyms
        .iter()
        .any(|&keysym| matches!(keysym, Keysym::Caps_Lock | Keysym::Num_Lock))
}

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
    fn the_escape_keysym_closes_the_overview() {
        assert!(closes_overview(&[Keysym::Escape]));
    }

    #[test]
    fn caps_lock_and_other_keysyms_do_not_close_the_overview() {
        assert!(!closes_overview(&[Keysym::Caps_Lock]));
        assert!(!closes_overview(&[Keysym::a]));
        assert!(!closes_overview(&[]));
    }

    #[test]
    fn lock_keys_pass_through_and_other_keys_do_not() {
        assert!(is_lock_key(&[Keysym::Caps_Lock]));
        assert!(is_lock_key(&[Keysym::Num_Lock]));
        assert!(!is_lock_key(&[Keysym::Escape]));
        assert!(!is_lock_key(&[Keysym::a]));
    }

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

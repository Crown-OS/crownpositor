//! What a key does while the overview owns the keyboard.

use smithay::input::keyboard::Keysym;

use crate::input::shortcuts::Action;

/// The physical Escape key closes the overview in both settings. With the swap
/// on, that key produces Caps Lock and the Caps Lock key produces Escape, so
/// both keysyms close it.
pub(super) fn closes_overview(keysyms: &[Keysym], swap_caps_and_esc: bool) -> bool {
    keysyms.iter().any(|&keysym| {
        keysym == Keysym::Escape || (swap_caps_and_esc && keysym == Keysym::Caps_Lock)
    })
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
    fn the_escape_keysym_closes_the_overview_in_both_settings() {
        assert!(closes_overview(&[Keysym::Escape], false));
        assert!(closes_overview(&[Keysym::Escape], true));
    }

    #[test]
    fn the_physical_escape_key_closes_the_overview_when_it_makes_caps_lock() {
        assert!(closes_overview(&[Keysym::Caps_Lock], true));
        assert!(!closes_overview(&[Keysym::Caps_Lock], false));
    }

    #[test]
    fn other_keysyms_do_not_close_the_overview() {
        assert!(!closes_overview(&[Keysym::a], true));
        assert!(!closes_overview(&[], true));
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

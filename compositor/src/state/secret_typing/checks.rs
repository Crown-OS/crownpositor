//! The decisions of a typing request that need no compositor state.

use protocols::crownos_secret_input::Outcome;

use crate::state::text_input_purposes::AppliedTextInput;

/// App ids of terminal emulators: text typed there is a command line.
const TERMINAL_APP_IDS: [&str; 16] = [
    "kgx",
    "org.gnome.console",
    "org.gnome.ptyxis",
    "foot",
    "footclient",
    "kitty",
    "alacritty",
    "konsole",
    "org.kde.konsole",
    "tilix",
    "com.gexperts.tilix",
    "terminator",
    "st",
    "urxvt",
    "com.mitchellh.ghostty",
    "io.elementary.terminal",
];

/// The longest display text the dialog shows for one field.
const DISPLAY_TEXT_CHARACTERS: usize = 120;

pub fn is_terminal_app(app_id: &str) -> bool {
    let app_id = app_id.to_ascii_lowercase();
    TERMINAL_APP_IDS.contains(&app_id.as_str())
        || app_id.contains("terminal")
        || app_id.ends_with("term")
}

/// The window must exist and carry the app id the daemon asked for.
pub fn check_target(requested_app_id: &str, window_app_id: Option<&str>) -> Result<(), Outcome> {
    let window_app_id = window_app_id.ok_or(Outcome::UnknownWindow)?;
    if requested_app_id.is_empty() || requested_app_id != window_app_id {
        return Err(Outcome::TargetMismatch);
    }
    Ok(())
}

/// What the focused field allows for the next value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCheck {
    Ready,
    Wait,
    Refuse(Outcome),
}

/// `enables_before` is the enable count of the same text input when Tab was
/// pressed: the field that Tab moved to must enable the text input again.
pub fn check_field(
    applied: Option<AppliedTextInput>,
    needs_password_field: bool,
    enables_before: Option<u32>,
) -> FieldCheck {
    let Some(applied) = applied else {
        return FieldCheck::Wait;
    };
    if enables_before.is_some_and(|before| applied.enables == before) || !applied.enabled {
        return FieldCheck::Wait;
    }
    if applied.is_terminal() {
        return FieldCheck::Refuse(Outcome::Refused);
    }
    if needs_password_field && !applied.is_password() {
        return FieldCheck::Wait;
    }
    FieldCheck::Ready
}

/// The answer when the field did not become ready in time.
pub fn outcome_after_wait(focus_on_window: bool, needs_password_field: bool) -> Outcome {
    if !focus_on_window {
        Outcome::FocusChanged
    } else if needs_password_field {
        Outcome::NotPasswordField
    } else {
        Outcome::Refused
    }
}

/// Caller text for the dialog: no control characters, and short.
pub fn display_text(text: &str) -> String {
    let mut shown: String = text
        .chars()
        .filter(|character| !character.is_control())
        .take(DISPLAY_TEXT_CHARACTERS + 1)
        .collect();
    if shown.chars().count() > DISPLAY_TEXT_CHARACTERS {
        shown = shown.chars().take(DISPLAY_TEXT_CHARACTERS - 1).collect();
        shown.push('…');
    }
    shown
}

#[cfg(test)]
mod tests {
    use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::ContentPurpose;

    use super::*;

    fn field(enabled: bool, purpose: Option<ContentPurpose>, enables: u32) -> AppliedTextInput {
        AppliedTextInput {
            enabled,
            purpose,
            enables,
        }
    }

    #[test]
    fn a_window_of_another_app_is_a_target_mismatch() {
        assert_eq!(
            check_target("org.mozilla.firefox", Some("org.mozilla.firefox")),
            Ok(())
        );
        assert_eq!(
            check_target("org.mozilla.firefox", Some("org.evil.app")),
            Err(Outcome::TargetMismatch)
        );
        assert_eq!(check_target("", Some("")), Err(Outcome::TargetMismatch));
        assert_eq!(
            check_target("org.mozilla.firefox", None),
            Err(Outcome::UnknownWindow)
        );
    }

    #[test]
    fn terminals_are_known_by_app_id() {
        for app_id in [
            "kitty",
            "org.gnome.Terminal",
            "Alacritty",
            "org.wezfurlong.wezterm",
            "xterm",
        ] {
            assert!(is_terminal_app(app_id), "{app_id}");
        }
        for app_id in ["chromium", "org.mozilla.firefox", "org.gnome.Nautilus"] {
            assert!(!is_terminal_app(app_id), "{app_id}");
        }
    }

    #[test]
    fn a_password_value_waits_for_a_password_field() {
        let password = field(true, Some(ContentPurpose::Password), 2);
        let normal = field(true, Some(ContentPurpose::Normal), 2);
        assert_eq!(check_field(Some(password), true, None), FieldCheck::Ready);
        assert_eq!(check_field(Some(normal), true, None), FieldCheck::Wait);
        assert_eq!(check_field(Some(normal), false, None), FieldCheck::Ready);
        assert_eq!(check_field(None, true, None), FieldCheck::Wait);
        assert_eq!(check_field(None, false, None), FieldCheck::Wait);
        let disabled = field(false, Some(ContentPurpose::Password), 2);
        assert_eq!(check_field(Some(disabled), true, None), FieldCheck::Wait);
    }

    #[test]
    fn after_tab_the_next_field_must_declare_itself() {
        let password = field(true, Some(ContentPurpose::Password), 4);
        assert_eq!(check_field(Some(password), true, Some(4)), FieldCheck::Wait);
        assert_eq!(
            check_field(Some(password), true, Some(3)),
            FieldCheck::Ready
        );
    }

    #[test]
    fn a_terminal_field_is_refused() {
        let terminal = field(true, Some(ContentPurpose::Terminal), 1);
        assert_eq!(
            check_field(Some(terminal), false, None),
            FieldCheck::Refuse(Outcome::Refused)
        );
    }

    #[test]
    fn a_wait_that_runs_out_names_what_was_missing() {
        assert_eq!(outcome_after_wait(false, true), Outcome::FocusChanged);
        assert_eq!(outcome_after_wait(true, true), Outcome::NotPasswordField);
        assert_eq!(outcome_after_wait(true, false), Outcome::Refused);
    }

    #[test]
    fn display_text_is_clean_and_short() {
        assert_eq!(display_text("Claude\n\u{1b}[2J"), "Claude[2J");
        let long = "a".repeat(500);
        let shown = display_text(&long);
        assert_eq!(shown.chars().count(), DISPLAY_TEXT_CHARACTERS);
        assert!(shown.ends_with('…'));
        assert_eq!(display_text("short"), "short");
    }
}

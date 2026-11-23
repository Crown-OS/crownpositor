//! The decisions of a typing request that need no compositor state.

use crownos_config::schema::AppKey;
use protocols::crownos_secret_input::Outcome;

use crate::state::text_input_purposes::AppliedTextInput;

/// App ids of terminal emulators and of editors with a terminal inside: text
/// typed there can be a command line.
const TERMINAL_APP_IDS: [&str; 30] = [
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
    "com.raggesilver.blackbox",
    "blackbox",
    "dev.warp.warp",
    "warp",
    "org.tabby",
    "tabby",
    "code",
    "code-oss",
    "codium",
    "com.visualstudio.code",
    "com.vscodium.codium",
    "cursor",
    "dev.zed.zed",
    "zed",
];

/// Executable names of the same programs, for a window whose app id says
/// something else.
const TERMINAL_EXECUTABLES: [&str; 22] = [
    "kgx",
    "gnome-terminal-server",
    "ptyxis",
    "foot",
    "kitty",
    "alacritty",
    "konsole",
    "tilix",
    "terminator",
    "st",
    "urxvt",
    "xterm",
    "wezterm-gui",
    "ghostty",
    "blackbox",
    "warp",
    "tabby",
    "code",
    "codium",
    "cursor",
    "zed",
    "zeditor",
];

/// The longest display text the dialog shows for one field.
const DISPLAY_TEXT_CHARACTERS: usize = 120;

fn looks_like_terminal(name: &str) -> bool {
    TERMINAL_APP_IDS.contains(&name)
        || name.contains("terminal")
        || name.ends_with("term")
        || name.starts_with("jetbrains-")
}

pub fn is_terminal(app_id: &str, app_key: &AppKey) -> bool {
    let executable = match app_key {
        AppKey::HostExecutable { path } => path.file_name(),
        AppKey::Interpreted { script, .. } => script.file_name(),
        AppKey::Sandboxed { .. } | AppKey::Spawned { .. } => None,
    }
    .and_then(|name| name.to_str())
    .map(str::to_ascii_lowercase);
    let sandboxed_app = match app_key {
        AppKey::Sandboxed { app_id, .. } => Some(app_id.to_ascii_lowercase()),
        _ => None,
    };
    looks_like_terminal(&app_id.to_ascii_lowercase())
        || sandboxed_app.is_some_and(|app| looks_like_terminal(&app))
        || executable.is_some_and(|name| {
            TERMINAL_EXECUTABLES.contains(&name.as_str()) || looks_like_terminal(&name)
        })
}

/// The window's client must have the app key that the daemon expects. A
/// client without a verified key never matches.
pub fn check_app_key(expected: &str, actual: Option<AppKey>) -> Result<AppKey, Outcome> {
    let expected: AppKey = expected.parse().map_err(|_| Outcome::TargetMismatch)?;
    actual
        .filter(|actual| *actual == expected)
        .ok_or(Outcome::TargetMismatch)
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

    fn exe(path: &str) -> AppKey {
        AppKey::HostExecutable { path: path.into() }
    }

    #[test]
    fn a_spoofed_app_id_does_not_change_the_verified_key() {
        let browser = exe("/usr/lib/chromium/chromium");
        assert_eq!(
            check_app_key("exe:/usr/lib/chromium/chromium", Some(browser.clone())),
            Ok(browser)
        );
        assert_eq!(
            check_app_key(
                "exe:/usr/lib/chromium/chromium",
                Some(exe("/home/user/evil"))
            ),
            Err(Outcome::TargetMismatch)
        );
        assert_eq!(
            check_app_key("exe:/usr/lib/chromium/chromium", None),
            Err(Outcome::TargetMismatch)
        );
        assert_eq!(
            check_app_key("not a key", Some(exe("/usr/bin/x"))),
            Err(Outcome::TargetMismatch)
        );
    }

    #[test]
    fn terminals_are_known_by_app_id_or_executable() {
        let plain = exe("/usr/lib/chromium/chromium");
        for app_id in [
            "kitty",
            "org.gnome.Terminal",
            "Alacritty",
            "org.wezfurlong.wezterm",
            "com.raggesilver.BlackBox",
            "dev.warp.Warp",
            "jetbrains-idea",
            "code",
        ] {
            assert!(is_terminal(app_id, &plain), "{app_id}");
        }
        for executable in [
            "/usr/bin/kitty",
            "/usr/bin/gnome-terminal-server",
            "/opt/tabby/tabby",
        ] {
            assert!(
                is_terminal("org.example.Browser", &exe(executable)),
                "{executable}"
            );
        }
        let flatpak = AppKey::Sandboxed {
            engine: "org.flatpak".into(),
            app_id: "com.raggesilver.BlackBox".into(),
        };
        assert!(is_terminal("bank", &flatpak));
        for app_id in ["chromium", "org.mozilla.firefox", "org.gnome.Nautilus"] {
            assert!(!is_terminal(app_id, &plain), "{app_id}");
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

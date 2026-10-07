//! The table a fresh install runs on.
//!
//! Typed rather than written as strings and parsed: a default cannot be a typo
//! that only surfaces as a panic on somebody else's machine.

use crownos_config::{KeyCode, Keybind, Mods};

use crate::input::shortcuts::action::{
    Action, Control, Direction, MediaCommand, SnapZone, Step, WorkspaceRef,
};

const NONE: Mods = Mods::NONE;
const SUPER: Mods = Mods::META;
const SUPER_SHIFT: Mods = Mods {
    shift: true,
    ..Mods::META
};
const ALT: Mods = Mods {
    alt: true,
    ..Mods::NONE
};
const ALT_SHIFT: Mods = Mods { shift: true, ..ALT };
const SUPER_CTRL: Mods = Mods {
    ctrl: true,
    ..Mods::META
};

const TERMINAL: &str = "kitty";

/// The workspaces reachable by their own digit, in order.
const WORKSPACE_KEYS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

const VOLUME_STEP: Step = Step(5);
const BRIGHTNESS_STEP: Step = Step(5);
/// Keyboard backlights mostly have three or four levels, so a step is a level.
const KEYBOARD_BRIGHTNESS_STEP: Step = Step(34);

/// The keys printed with a speaker, a sun or a transport symbol, unmodified.
const HARDWARE_KEYS: [(KeyCode, Control); 12] = [
    (KeyCode::AudioVolumeUp, Control::Volume(VOLUME_STEP)),
    (
        KeyCode::AudioVolumeDown,
        Control::Volume(Step(-VOLUME_STEP.0)),
    ),
    (KeyCode::AudioVolumeMute, Control::ToggleMute),
    (KeyCode::MicrophoneMute, Control::ToggleMicMute),
    (KeyCode::BrightnessUp, Control::Brightness(BRIGHTNESS_STEP)),
    (
        KeyCode::BrightnessDown,
        Control::Brightness(Step(-BRIGHTNESS_STEP.0)),
    ),
    (
        KeyCode::KeyboardBrightnessUp,
        Control::KeyboardBrightness(KEYBOARD_BRIGHTNESS_STEP),
    ),
    (
        KeyCode::KeyboardBrightnessDown,
        Control::KeyboardBrightness(Step(-KEYBOARD_BRIGHTNESS_STEP.0)),
    ),
    (
        KeyCode::MediaPlayPause,
        Control::Media(MediaCommand::PlayPause),
    ),
    (KeyCode::MediaStop, Control::Media(MediaCommand::Stop)),
    (KeyCode::MediaTrackNext, Control::Media(MediaCommand::Next)),
    (
        KeyCode::MediaTrackPrevious,
        Control::Media(MediaCommand::Previous),
    ),
];

/// Every built-in binding.
///
/// `Super+Space` is left alone: `keybinds.launcher` defaults to it, and a
/// compositor binding on the same chord would shadow the launcher.
pub fn bindings() -> impl Iterator<Item = (Keybind, Action)> {
    let fixed = [
        (SUPER_SHIFT, KeyCode::E, Action::Quit),
        (SUPER_SHIFT, KeyCode::C, Action::ReloadConfig),
        (
            SUPER,
            KeyCode::Enter,
            Action::Spawn(vec![TERMINAL.to_owned()]),
        ),
        (SUPER, KeyCode::Q, Action::CloseWindow),
        (SUPER, KeyCode::E, Action::ToggleWorkspaceView),
        (ALT, KeyCode::Tab, Action::WindowControlNext),
        (ALT_SHIFT, KeyCode::Tab, Action::WindowControlPrevious),
        (SUPER, KeyCode::H, Action::Focus(Direction::Left)),
        (SUPER, KeyCode::L, Action::Focus(Direction::Right)),
        (SUPER, KeyCode::K, Action::Focus(Direction::Up)),
        (SUPER, KeyCode::J, Action::Focus(Direction::Down)),
        (SUPER_SHIFT, KeyCode::H, Action::MoveWindow(Direction::Left)),
        (
            SUPER_SHIFT,
            KeyCode::L,
            Action::MoveWindow(Direction::Right),
        ),
        (SUPER_SHIFT, KeyCode::K, Action::MoveWindow(Direction::Up)),
        (SUPER_SHIFT, KeyCode::J, Action::MoveWindow(Direction::Down)),
        (
            SUPER,
            KeyCode::Tab,
            Action::Workspace(WorkspaceRef::Relative(1)),
        ),
        (
            SUPER_SHIFT,
            KeyCode::Tab,
            Action::Workspace(WorkspaceRef::Relative(-1)),
        ),
        (SUPER, KeyCode::V, Action::ToggleFloating),
        (SUPER, KeyCode::F, Action::ToggleFullscreen),
        (SUPER, KeyCode::M, Action::ToggleMaximize),
        (SUPER_SHIFT, KeyCode::Space, Action::ToggleWorkspaceMode),
        (SUPER_CTRL, KeyCode::L, Action::ResizeSplit(0.05)),
        (SUPER_CTRL, KeyCode::H, Action::ResizeSplit(-0.05)),
        (SUPER, KeyCode::P, Action::PromoteDemote),
        (SUPER_SHIFT, KeyCode::R, Action::ResetSize),
        // The three edges Windows snaps to, on the arrows that point at them.
        (
            SUPER,
            KeyCode::ArrowLeft,
            Action::SnapWindow(SnapZone::LeftHalf),
        ),
        (
            SUPER,
            KeyCode::ArrowRight,
            Action::SnapWindow(SnapZone::RightHalf),
        ),
        (
            SUPER,
            KeyCode::ArrowUp,
            Action::SnapWindow(SnapZone::Maximize),
        ),
    ]
    .into_iter()
    .map(|(mods, key, action)| (Keybind::new(mods, Some(key)), action));

    // Derived from the digit's position so the label and the workspace it
    // reaches cannot drift apart.
    let workspaces = WORKSPACE_KEYS
        .into_iter()
        .enumerate()
        .flat_map(|(index, key)| {
            [
                (
                    Keybind::new(SUPER, Some(key)),
                    Action::Workspace(WorkspaceRef::Index(index)),
                ),
                (
                    Keybind::new(SUPER_SHIFT, Some(key)),
                    Action::MoveWindowToWorkspace {
                        target: WorkspaceRef::Index(index),
                        follow: true,
                    },
                ),
            ]
        });

    let hardware = HARDWARE_KEYS
        .into_iter()
        .map(|(key, control)| (Keybind::new(NONE, Some(key)), Action::Control(control)));

    fixed.chain(workspaces).chain(hardware)
}

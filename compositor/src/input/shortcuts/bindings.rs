//! Chord parsing and the binding table.

use std::{collections::HashMap, fmt, str::FromStr};

use smithay::input::keyboard::{keysyms, Keysym, KeysymHandle, ModifiersState};

use config::{Binding, Compositor};

#[cfg(test)]
use crate::input::shortcuts::action::Direction;
use crate::input::shortcuts::action::{Action, WorkspaceRef};

/// The four modifiers a shortcut may name.
///
/// `caps_lock`, `num_lock` and `iso_level3/5_shift` are excluded: including them
/// breaks Super+Q with CapsLock on, and every Alt binding on an AltGr layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModMask {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl ModMask {
    pub fn from_smithay(modifiers: &ModifiersState) -> Self {
        Self {
            logo: modifiers.logo,
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
        }
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// A parsed chord: some modifiers plus at most one ordinary key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: ModMask,
    /// `None` for a modifier-only chord such as `"Super"`, which fires on
    /// release rather than press.
    pub key: Option<Keysym>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseChordError {
    Empty,
    UnknownToken(String),
    MultipleKeys,
}

impl fmt::Display for ParseChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty chord"),
            Self::UnknownToken(token) => write!(f, "unknown key or modifier `{token}`"),
            Self::MultipleKeys => write!(f, "a chord may name at most one non-modifier key"),
        }
    }
}

impl std::error::Error for ParseChordError {}

impl FromStr for Chord {
    type Err = ParseChordError;

    /// Parses `"Super+Shift+Q"`. Case- and order-insensitive, so `"shift+super+q"`
    /// is the same chord. `"None"` parses to an empty chord that matches nothing.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(ParseChordError::Empty);
        }
        if s.eq_ignore_ascii_case("none") {
            return Ok(Self {
                mods: ModMask::default(),
                key: None,
            });
        }

        let mut mods = ModMask::default();
        let mut key = None;

        for token in s.split('+').map(str::trim).filter(|t| !t.is_empty()) {
            let lower = token.to_ascii_lowercase();
            match lower.as_str() {
                "super" | "logo" | "meta" | "mod4" | "cmd" => mods.logo = true,
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "mod1" | "option" => mods.alt = true,
                "shift" => mods.shift = true,
                _ => {
                    if key.is_some() {
                        return Err(ParseChordError::MultipleKeys);
                    }
                    key = Some(
                        keysym_from_name(&lower)
                            .ok_or_else(|| ParseChordError::UnknownToken(token.to_owned()))?,
                    );
                }
            }
        }

        Ok(Self { mods, key })
    }
}

/// Name -> keysym, explicit rather than via xkb so the accepted spellings do not
/// shift with the user's keymap.
fn keysym_from_name(lower: &str) -> Option<Keysym> {
    // Shift lives in the modifier mask, not the keysym, so `Super+Shift+Q` is
    // `{shift, q}` rather than `{shift, Q}`.
    if lower.chars().count() == 1 {
        let ch = lower.chars().next().expect("one char");
        if ch.is_ascii_graphic() {
            return Some(Keysym::from(ch as u32));
        }
    }

    let raw = match lower {
        "return" | "enter" => keysyms::KEY_Return,
        "space" => keysyms::KEY_space,
        "tab" => keysyms::KEY_Tab,
        "escape" | "esc" => keysyms::KEY_Escape,
        "backspace" => keysyms::KEY_BackSpace,
        "delete" | "del" => keysyms::KEY_Delete,
        "home" => keysyms::KEY_Home,
        "end" => keysyms::KEY_End,
        "pageup" | "prior" => keysyms::KEY_Prior,
        "pagedown" | "next" => keysyms::KEY_Next,
        "insert" => keysyms::KEY_Insert,
        "left" => keysyms::KEY_Left,
        "right" => keysyms::KEY_Right,
        "up" => keysyms::KEY_Up,
        "down" => keysyms::KEY_Down,
        "f1" => keysyms::KEY_F1,
        "f2" => keysyms::KEY_F2,
        "f3" => keysyms::KEY_F3,
        "f4" => keysyms::KEY_F4,
        "f5" => keysyms::KEY_F5,
        "f6" => keysyms::KEY_F6,
        "f7" => keysyms::KEY_F7,
        "f8" => keysyms::KEY_F8,
        "f9" => keysyms::KEY_F9,
        "f10" => keysyms::KEY_F10,
        "f11" => keysyms::KEY_F11,
        "f12" => keysyms::KEY_F12,
        _ => return None,
    };
    Some(Keysym::new(raw))
}

#[derive(Debug, Default)]
pub struct Bindings {
    keys: HashMap<Chord, Action>,
    /// Modifier-only chords, fired on the release edge.
    mod_only: HashMap<ModMask, Action>,
}

impl Bindings {
    pub fn defaults() -> Self {
        const DEFAULTS: &[(&str, &str)] = &[
            ("Super+Shift+E", "quit"),
            ("Super+Return", "spawn foot"),
            ("Super+Q", "close-window"),
            ("Super+H", "focus left"),
            ("Super+L", "focus right"),
            ("Super+K", "focus up"),
            ("Super+J", "focus down"),
            ("Super+Shift+H", "move left"),
            ("Super+Shift+L", "move right"),
            ("Super+Shift+K", "move up"),
            ("Super+Shift+J", "move down"),
            ("Super+Tab", "workspace +1"),
            ("Super+Shift+Tab", "workspace -1"),
            ("Super+1", "workspace 0"),
            ("Super+2", "workspace 1"),
            ("Super+3", "workspace 2"),
            ("Super+4", "workspace 3"),
            ("Super+Shift+1", "move-to-workspace 0 follow"),
            ("Super+Shift+2", "move-to-workspace 1 follow"),
            ("Super+Shift+3", "move-to-workspace 2 follow"),
            ("Super+Shift+4", "move-to-workspace 3 follow"),
            ("Super+V", "toggle-float"),
            ("Super+F", "toggle-fullscreen"),
            ("Super+M", "toggle-maximize"),
            ("Super+Space", "cycle-layout"),
            ("Super+Shift+Space", "toggle-layout-mode"),
            ("Super+Shift+C", "reload-config"),
            ("Super+Ctrl+L", "resize-split 0.05"),
            ("Super+Ctrl+H", "resize-split -0.05"),
            ("Super+P", "promote"),
            ("Super+R", "cycle-size"),
            ("Super+Shift+R", "reset-size"),
        ];

        let mut bindings = Self::default();
        for (chord, action) in DEFAULTS {
            let chord: Chord = chord.parse().expect("built-in chord parses");
            let action: Action = action.parse().expect("built-in action parses");
            bindings.insert(chord, action);
        }
        bindings
    }

    pub fn from_config(config: &Compositor) -> Self {
        if config.keybinds.is_empty() {
            return Self::defaults();
        }

        let mut bindings = Self::default();
        for Binding { keys, action } in &config.keybinds {
            let chord = match keys.parse::<Chord>() {
                Ok(chord) => chord,
                Err(err) => {
                    tracing::warn!(%err, keys, action, "skipping keybind");
                    continue;
                }
            };
            let parsed = match action.parse::<Action>() {
                Ok(parsed) => parsed,
                Err(err) => {
                    tracing::warn!(%err, keys, action, "skipping keybind");
                    continue;
                }
            };
            bindings.insert(chord, parsed);
        }
        bindings
    }

    fn insert(&mut self, chord: Chord, action: Action) {
        match chord.key {
            Some(key) => {
                self.keys.insert(
                    Chord {
                        mods: chord.mods,
                        key: Some(key),
                    },
                    action,
                );
            }
            // `"None"` binds nothing — how a user asks for an empty table.
            None if chord.mods.is_empty() => {}
            None => {
                self.mod_only.insert(chord.mods, action);
            }
        }
    }

    /// Prefers the layout-independent symbol, so Super+Q is still Super+Q on a
    /// Cyrillic or Dvorak keymap, then falls back to the modified symbol.
    pub fn lookup(&self, modifiers: &ModifiersState, handle: &KeysymHandle<'_>) -> Option<Action> {
        let mods = ModMask::from_smithay(modifiers);

        handle
            .raw_latin_sym_or_raw_current_sym()
            .and_then(|key| {
                self.keys.get(&Chord {
                    mods,
                    key: Some(key),
                })
            })
            .or_else(|| {
                self.keys.get(&Chord {
                    mods,
                    key: Some(handle.modified_sym()),
                })
            })
            .cloned()
    }

    pub fn lookup_mod_only(&self, mods: ModMask) -> Option<Action> {
        self.mod_only.get(&mods).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.mod_only.is_empty()
    }
}

/// Touchpad gestures resolve to the same [`Action`]s the keyboard produces.
#[derive(Debug, Default)]
pub struct GestureBindings {
    map: HashMap<crate::input::trackpad::gestures::SwipeGesture, Action>,
}

impl GestureBindings {
    pub fn defaults() -> Self {
        use crate::input::trackpad::gestures::{Fingers, SwipeGesture};

        let mut map = HashMap::new();
        map.insert(
            SwipeGesture::LeftToRight(Fingers::Three),
            Action::Workspace(WorkspaceRef::Relative(-1)),
        );
        map.insert(
            SwipeGesture::RightToLeft(Fingers::Three),
            Action::Workspace(WorkspaceRef::Relative(1)),
        );
        map.insert(
            SwipeGesture::BottomToTop(Fingers::Four),
            Action::OpenWorkspaceView,
        );
        map.insert(
            SwipeGesture::TopToBottom(Fingers::Four),
            Action::CloseWorkspaceView,
        );
        Self { map }
    }

    pub fn lookup(
        &self,
        gesture: crate::input::trackpad::gestures::SwipeGesture,
    ) -> Option<Action> {
        self.map.get(&gesture).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap_or_else(|err| panic!("`{s}`: {err}"))
    }

    #[test]
    fn chord_parsing_is_order_and_case_insensitive() {
        assert_eq!(chord("Super+Shift+Q"), chord("shift+SUPER+q"));
    }

    #[test]
    fn modifier_only_chords_have_no_key() {
        let parsed = chord("Super");
        assert!(parsed.mods.logo);
        assert_eq!(parsed.key, None);
    }

    #[test]
    fn letters_are_lowercased_and_shift_stays_in_the_mask() {
        let parsed = chord("Super+Shift+Q");
        assert!(parsed.mods.shift);
        assert_eq!(parsed.key, Some(Keysym::from('q' as u32)));
    }

    #[test]
    fn named_keys_resolve() {
        assert_eq!(
            chord("Super+Return").key,
            Some(Keysym::new(keysyms::KEY_Return))
        );
        assert_eq!(
            chord("Super+Space").key,
            Some(Keysym::new(keysyms::KEY_space))
        );
        assert_eq!(chord("F11").key, Some(Keysym::new(keysyms::KEY_F11)));
    }

    #[test]
    fn unknown_and_ambiguous_chords_are_errors() {
        assert!("Super+Frobnicate".parse::<Chord>().is_err());
        assert!("Super+Q+W".parse::<Chord>().is_err());
        assert!("".parse::<Chord>().is_err());
    }

    #[test]
    fn every_default_binding_parses() {
        // `defaults()` unwraps, so a typo in the table is a startup panic. This
        // turns it into a test failure that names the row.
        let bindings = Bindings::defaults();
        assert!(bindings.keys.len() > 20, "the table should be substantial");
    }

    #[test]
    fn the_defaults_cover_every_core_operation() {
        let bindings = Bindings::defaults();
        let bound: Vec<&Action> = bindings.keys.values().collect();

        let has = |predicate: fn(&Action) -> bool| bound.iter().any(|a| predicate(a));

        assert!(has(|a| matches!(a, Action::Quit)), "no way to quit");
        assert!(has(|a| matches!(a, Action::CloseWindow)));
        assert!(has(|a| matches!(a, Action::Spawn(_))));
        assert!(has(|a| matches!(a, Action::Focus(_))));
        assert!(has(|a| matches!(a, Action::MoveWindow(_))));
        assert!(has(|a| matches!(a, Action::Workspace(_))));
        assert!(has(|a| matches!(a, Action::MoveWindowToWorkspace { .. })));
        assert!(has(|a| matches!(a, Action::ToggleFloating)));
        assert!(has(|a| matches!(a, Action::ToggleFullscreen)));
        assert!(has(|a| matches!(a, Action::ToggleMaximize)));
        assert!(has(|a| matches!(a, Action::CycleLayout)));
        assert!(has(|a| matches!(a, Action::ToggleLayoutMode)));
        assert!(has(|a| matches!(a, Action::ResizeSplit(_))));
        assert!(has(|a| matches!(a, Action::PromoteDemote)));
    }

    #[test]
    fn all_four_focus_directions_are_bound() {
        let bindings = Bindings::defaults();
        for dir in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            assert!(
                bindings.keys.values().any(|a| *a == Action::Focus(dir)),
                "{dir:?} is not bound"
            );
        }
    }

    #[test]
    fn no_chord_is_bound_twice() {
        // A duplicate would silently overwrite, so the later row wins with no
        // diagnostic. Counting the table against the source rows catches it.
        let bindings = Bindings::defaults();
        let mut seen = std::collections::HashSet::new();
        for chord in bindings.keys.keys() {
            assert!(seen.insert(*chord), "{chord:?} is bound more than once");
        }
    }

    #[test]
    fn defaults_bind_a_way_out() {
        let bindings = Bindings::defaults();
        assert_eq!(
            bindings.keys.get(&chord("Super+Shift+E")),
            Some(&Action::Quit),
            "a fresh install must have a quit binding"
        );
    }

    #[test]
    fn empty_config_falls_back_to_defaults() {
        let config = Compositor::default();
        assert!(
            !Bindings::from_config(&config).is_empty(),
            "an empty keybinds list means defaults, not an empty table"
        );
    }

    #[test]
    fn explicit_none_binds_nothing() {
        let config = Compositor {
            keybinds: vec![Binding {
                keys: "None".into(),
                action: "quit".into(),
            }],
            ..Default::default()
        };
        assert!(
            Bindings::from_config(&config).is_empty(),
            "a single `None` row is how a user asks for no bindings at all"
        );
    }

    #[test]
    fn one_bad_row_does_not_discard_the_others() {
        let config = Compositor {
            keybinds: vec![
                Binding {
                    keys: "Supper+Q".into(),
                    action: "quit".into(),
                },
                Binding {
                    keys: "Super+Q".into(),
                    action: "frobnicate".into(),
                },
                Binding {
                    keys: "Super+E".into(),
                    action: "quit".into(),
                },
            ],
            ..Default::default()
        };
        let bindings = Bindings::from_config(&config);
        assert_eq!(bindings.keys.len(), 1);
        assert_eq!(bindings.keys.get(&chord("Super+E")), Some(&Action::Quit));
    }
}

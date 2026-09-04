//! The binding table: the defaults, whatever the config lays over them, and
//! lookup from a key event.

use std::collections::HashMap;

use smithay::input::keyboard::{KeysymHandle, ModifiersState};

use config::Binding;

use crate::input::shortcuts::{
    action::Action,
    chord::{Chord, ModMask},
    custom::{self, Override},
    defaults,
};

#[derive(Debug, Default)]
pub struct Bindings {
    keys: HashMap<Chord, Action>,
    /// Modifier-only chords, fired on the release edge.
    mod_only: HashMap<ModMask, Action>,
}

impl Bindings {
    pub fn defaults() -> Self {
        let mut bindings = Self::default();
        for (bind, action) in defaults::bindings() {
            bindings.bind(bind.into(), action);
        }
        bindings
    }

    /// The defaults with the config's rows laid over them.
    ///
    /// A row rebinds one chord and leaves every other alone, so adding a
    /// shortcut cannot cost the user their way out of the session. Binding
    /// `none` deletes a chord instead, which hands the key back to whatever has
    /// focus — the only way to reach an application's own `Super+L`.
    pub fn with_custom(custom: &[Binding]) -> Self {
        let mut bindings = Self::defaults();

        for Override { chord, action } in custom::overrides(custom) {
            if chord.is_empty() {
                tracing::debug!(?action, "a keybind with no shortcut binds nothing");
            } else if matches!(action, Action::None) {
                bindings.unbind(chord);
            } else {
                bindings.bind(chord, action);
            }
        }

        bindings
    }

    fn bind(&mut self, chord: Chord, action: Action) {
        if chord.key.is_some() {
            self.keys.insert(chord, action);
        } else {
            self.mod_only.insert(chord.mods, action);
        }
    }

    fn unbind(&mut self, chord: Chord) {
        if chord.key.is_some() {
            self.keys.remove(&chord);
        } else {
            self.mod_only.remove(&chord.mods);
        }
    }

    /// Prefers the layout-independent symbol, so Super+Q is still Super+Q on a
    /// Cyrillic or Dvorak keymap, then falls back to the modified symbol.
    pub fn lookup(&self, modifiers: &ModifiersState, handle: &KeysymHandle<'_>) -> Option<Action> {
        let mods = ModMask::from_smithay(modifiers);
        let at = |key| {
            self.keys.get(&Chord {
                mods,
                key: Some(key),
            })
        };

        handle
            .raw_latin_sym_or_raw_current_sym()
            .and_then(at)
            .or_else(|| at(handle.modified_sym()))
            .cloned()
    }

    pub fn lookup_mod_only(&self, mods: ModMask) -> Option<Action> {
        self.mod_only.get(&mods).cloned()
    }
}

#[cfg(test)]
mod tests {
    use crownos_config::Keybind;

    use super::*;
    use crate::input::shortcuts::action::{Direction, WorkspaceRef};

    fn chord(written: &str) -> Chord {
        written
            .parse::<Keybind>()
            .unwrap_or_else(|err| panic!("`{written}`: {err}"))
            .into()
    }

    fn row(keys: &str, action: &str) -> Binding {
        Binding {
            keys: keys.to_owned(),
            action: action.to_owned(),
        }
    }

    fn bound(bindings: &Bindings, keys: &str) -> Option<Action> {
        bindings.keys.get(&chord(keys)).cloned()
    }

    #[test]
    fn the_defaults_cover_every_core_operation() {
        let bindings = Bindings::defaults();
        let has = |predicate: fn(&Action) -> bool| bindings.keys.values().any(predicate);

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

        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            assert!(
                bindings
                    .keys
                    .values()
                    .any(|a| *a == Action::Focus(direction)),
                "{direction:?} is not bound"
            );
        }
    }

    #[test]
    fn defaults_bind_a_way_out() {
        assert_eq!(
            bound(&Bindings::defaults(), "Super+Shift+E"),
            Some(Action::Quit),
            "a fresh install must have a quit binding"
        );
    }

    /// A digit reaches the workspace it is written on, not the one after it.
    #[test]
    fn the_workspace_digits_line_up_with_their_index() {
        let bindings = Bindings::defaults();

        assert_eq!(
            bound(&bindings, "Super+1"),
            Some(Action::Workspace(WorkspaceRef::Index(0)))
        );
        assert_eq!(
            bound(&bindings, "Super+Shift+5"),
            Some(Action::MoveWindowToWorkspace {
                target: WorkspaceRef::Index(4),
                follow: true,
            })
        );
    }

    /// Two defaults on one chord would let the later row win silently.
    #[test]
    fn no_default_chord_is_bound_twice() {
        let bindings = Bindings::defaults();

        assert_eq!(
            defaults::bindings().count(),
            bindings.keys.len() + bindings.mod_only.len()
        );
    }

    #[test]
    fn an_empty_config_leaves_the_defaults_alone() {
        assert_eq!(
            bound(&Bindings::with_custom(&[]), "Super+Q"),
            Some(Action::CloseWindow)
        );
    }

    #[test]
    fn a_custom_row_adds_without_costing_a_default() {
        let bindings = Bindings::with_custom(&[row("Super+B", "spawn firefox")]);

        assert_eq!(
            bound(&bindings, "Super+B"),
            Some(Action::Spawn(vec!["firefox".to_owned()]))
        );
        assert_eq!(
            bound(&bindings, "Super+Shift+E"),
            Some(Action::Quit),
            "one custom row must not discard the built-in table"
        );
    }

    #[test]
    fn a_custom_row_replaces_the_default_on_the_same_chord() {
        let bindings = Bindings::with_custom(&[row("Super+Enter", "spawn alacritty")]);

        assert_eq!(
            bound(&bindings, "Super+Enter"),
            Some(Action::Spawn(vec!["alacritty".to_owned()]))
        );
    }

    /// Deleted rather than bound to a no-op, so the key reaches the client.
    #[test]
    fn binding_none_gives_the_chord_back_to_the_client() {
        let bindings = Bindings::with_custom(&[row("Super+L", "none")]);

        assert_eq!(bound(&bindings, "Super+L"), None);
        assert_eq!(
            bound(&bindings, "Super+H"),
            Some(Action::Focus(Direction::Left)),
            "unbinding one chord must not touch its neighbours"
        );
    }

    #[test]
    fn a_modifier_only_row_fires_on_its_own_mask() {
        let bindings = Bindings::with_custom(&[row("Super+Ctrl", "spawn crownlauncher")]);

        assert_eq!(
            bindings.lookup_mod_only(chord("Super+Ctrl").mods),
            Some(Action::Spawn(vec!["crownlauncher".to_owned()]))
        );
        assert_eq!(bindings.lookup_mod_only(chord("Super+Alt").mods), None);
    }

    #[test]
    fn a_row_with_no_shortcut_is_ignored() {
        let bindings = Bindings::with_custom(&[row("None", "quit")]);

        assert_eq!(
            bound(&bindings, "Super+Shift+E"),
            Some(Action::Quit),
            "an unbound row says nothing about the rest of the table"
        );
    }

    #[test]
    fn the_last_row_on_a_chord_wins() {
        let bindings = Bindings::with_custom(&[
            row("Super+B", "spawn firefox"),
            row("Super+B", "spawn chromium"),
        ]);

        assert_eq!(
            bound(&bindings, "Super+B"),
            Some(Action::Spawn(vec!["chromium".to_owned()]))
        );
    }
}

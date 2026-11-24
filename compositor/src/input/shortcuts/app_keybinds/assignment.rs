//! Which chord each registered action gets.
//!
//! The chords the user set in `keybinds.app_keybinds` are reserved first,
//! also for applications that are not running, so a chord the user gave to
//! one action never goes to another. Then an action that already has the
//! chord its application asked for keeps it, so a chord does not move from
//! one running application to another. After that, each action that has no
//! user chord gets the chord its application asked for, in the order the
//! actions were registered, if no one else has it and it holds Super, Ctrl
//! or Alt. Without one of them the chord would take a typing key away from
//! every window.

use std::collections::{HashMap, HashSet};

use config::AppKeybind;
use crownos_config::Keybind;

use crate::input::shortcuts::chord::Chord;

/// What one registration asks for.
#[derive(Debug, Clone, Copy)]
pub struct ActionRequest<'a> {
    pub app_id: &'a str,
    pub name: &'a str,
    pub preferred_keys: &'a str,
    pub assigned_keys: Option<&'a str>,
}

impl ActionRequest<'_> {
    fn preferred_chord(&self) -> Option<Keybind> {
        let preferred = self
            .preferred_keys
            .parse::<Keybind>()
            .inspect_err(|error| {
                tracing::warn!(
                    %error,
                    app_id = self.app_id,
                    name = self.name,
                    "the application asked for a chord that cannot be read"
                );
            })
            .ok()?;
        let mods = preferred.mods;
        (mods.meta || mods.ctrl || mods.alt).then_some(preferred)
    }

    fn has_preferred_chord(&self) -> bool {
        let assigned = self
            .assigned_keys
            .and_then(|keys| keys.parse::<Keybind>().ok());
        assigned.is_some_and(|assigned| {
            !assigned.is_empty() && Ok(assigned) == self.preferred_keys.parse::<Keybind>()
        })
    }
}

type ActionIdentity<'a> = (&'a str, &'a str);

/// The chord of each request, in the order of `requests`. Requests for the
/// same action get the same chord. `is_desktop_chord` tells which chords the
/// compositor keeps for itself.
pub fn assign_chords(
    requests: &[ActionRequest<'_>],
    user_chords: &[AppKeybind],
    is_desktop_chord: impl Fn(Chord) -> bool,
) -> Vec<Option<Keybind>> {
    let mut used_chords = HashSet::new();
    let mut claim = |keybind: Keybind| {
        let chord = Chord::from(keybind);
        (!keybind.is_empty() && !is_desktop_chord(chord) && used_chords.insert(chord))
            .then_some(keybind)
    };

    let mut chord_of_action: HashMap<ActionIdentity<'_>, Option<Keybind>> = HashMap::new();
    for user_chord in user_chords {
        chord_of_action
            .entry((user_chord.app_id.as_str(), user_chord.name.as_str()))
            .or_insert_with(|| claim(user_chord.keys));
    }
    let (holders, newcomers): (Vec<&ActionRequest<'_>>, Vec<_>) = requests
        .iter()
        .partition(|request| request.has_preferred_chord());
    for request in holders.into_iter().chain(newcomers) {
        chord_of_action
            .entry((request.app_id, request.name))
            .or_insert_with(|| request.preferred_chord().and_then(&mut claim));
    }

    requests
        .iter()
        .map(|request| {
            chord_of_action
                .get(&(request.app_id, request.name))
                .copied()
                .flatten()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(app_id: &'a str, name: &'a str, preferred_keys: &'a str) -> ActionRequest<'a> {
        ActionRequest {
            app_id,
            name,
            preferred_keys,
            assigned_keys: None,
        }
    }

    fn user_chord(app_id: &str, name: &str, keys: &str) -> AppKeybind {
        AppKeybind {
            app_id: app_id.to_owned(),
            name: name.to_owned(),
            keys: keys
                .parse()
                .unwrap_or_else(|error| panic!("`{keys}`: {error}")),
        }
    }

    fn written(chords: Vec<Option<Keybind>>) -> Vec<String> {
        chords
            .into_iter()
            .map(|chord| chord.unwrap_or(Keybind::NONE).to_string())
            .collect()
    }

    fn no_desktop_chords(_: Chord) -> bool {
        false
    }

    #[test]
    fn a_free_chord_goes_to_the_application_that_asked_for_it() {
        let chords = assign_chords(
            &[request("recorder", "toggle", "Super+Alt+R")],
            &[],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["Super+Alt+R"]);
    }

    #[test]
    fn the_user_chord_replaces_the_one_the_application_asked_for() {
        let chords = assign_chords(
            &[request("recorder", "toggle", "Super+Alt+R")],
            &[user_chord("recorder", "toggle", "Ctrl+F9")],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["Ctrl+F9"]);
    }

    #[test]
    fn the_user_can_remove_the_chord_of_an_action() {
        let chords = assign_chords(
            &[request("recorder", "toggle", "Super+Alt+R")],
            &[user_chord("recorder", "toggle", "None")],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["None"]);
    }

    #[test]
    fn the_first_application_to_ask_keeps_the_chord() {
        let chords = assign_chords(
            &[
                request("recorder", "toggle", "Super+Alt+R"),
                request("camera", "shutter", "Super+Alt+R"),
            ],
            &[],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["Super+Alt+R", "None"]);
    }

    #[test]
    fn a_user_chord_stays_reserved_while_its_application_is_not_running() {
        let chords = assign_chords(
            &[request("camera", "shutter", "Super+Alt+R")],
            &[user_chord("recorder", "toggle", "Super+Alt+R")],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["None"]);
    }

    #[test]
    fn the_desktop_keeps_its_own_chords() {
        let desktop_chord = Chord::from("Super+Q".parse::<Keybind>().unwrap());
        let chords = assign_chords(
            &[request("recorder", "toggle", "Super+Q")],
            &[user_chord("camera", "shutter", "super+q")],
            |chord| chord == desktop_chord,
        );
        assert_eq!(written(chords), ["None"]);
    }

    #[test]
    fn unreadable_and_empty_chords_are_never_assigned() {
        let chords = assign_chords(
            &[
                request("recorder", "toggle", "Supper+R"),
                request("camera", "shutter", ""),
                request("player", "pause", "None"),
            ],
            &[],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["None", "None", "None"]);
    }

    #[test]
    fn an_application_can_hold_a_modifier_only_chord() {
        let chords = assign_chords(
            &[
                ActionRequest {
                    assigned_keys: Some("Super"),
                    ..request("assistant", "talk", "Super")
                },
                request("dictation", "talk", "Super"),
                request("logger", "shift", "Shift"),
            ],
            &[],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["Super", "None", "None"]);
    }

    #[test]
    fn the_desktop_keeps_its_modifier_only_chords() {
        let desktop_chord = Chord::from("Super+Ctrl".parse::<Keybind>().unwrap());
        let chords = assign_chords(
            &[request("assistant", "talk", "Super+Ctrl")],
            &[],
            |chord| chord == desktop_chord,
        );
        assert_eq!(written(chords), ["None"]);
    }

    #[test]
    fn a_running_application_keeps_its_chord_when_an_older_one_wants_it_back() {
        let chords = assign_chords(
            &[
                ActionRequest {
                    assigned_keys: Some("Super+Alt+Q"),
                    ..request("recorder", "toggle", "Super+Alt+R")
                },
                ActionRequest {
                    assigned_keys: Some("Super+Alt+R"),
                    ..request("camera", "shutter", "super+alt+r")
                },
            ],
            &[],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["None", "Super+Alt+R"]);
    }

    #[test]
    fn an_application_cannot_take_a_typing_key() {
        let chords = assign_chords(
            &[
                request("logger", "letter", "R"),
                request("logger", "capital", "Shift+R"),
                request("logger", "space", "Space"),
            ],
            &[],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["None", "None", "None"]);
    }

    #[test]
    fn the_user_can_give_an_action_a_key_without_modifiers() {
        let chords = assign_chords(
            &[request("recorder", "toggle", "Super+Alt+R")],
            &[user_chord("recorder", "toggle", "F9")],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["F9"]);
    }

    #[test]
    fn two_registrations_of_one_action_share_its_chord() {
        let chords = assign_chords(
            &[
                request("recorder", "toggle", "Super+Alt+R"),
                request("recorder", "toggle", "Super+Alt+T"),
            ],
            &[],
            no_desktop_chords,
        );
        assert_eq!(written(chords), ["Super+Alt+R", "Super+Alt+R"]);
    }
}

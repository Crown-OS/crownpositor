//! The list of registered actions that the compositor writes to
//! `app_actions.ron`, so the settings panel can show them.

use config::AppAction;

/// Records the actions that are registered now in `known`, the list from the
/// last write. An action that is not registered now stays in the list as not
/// running. When two registrations have the same action, the last one wins.
pub fn record_registered_actions(
    known: &mut Vec<AppAction>,
    registered: impl IntoIterator<Item = AppAction>,
) {
    for action in known.iter_mut() {
        action.running = false;
    }
    for action in registered {
        match known
            .iter_mut()
            .find(|known| known.is_action(&action.app_id, &action.name))
        {
            Some(known) => *known = action,
            None => known.push(action),
        }
    }
}

#[cfg(test)]
mod tests {
    use crownos_config::Keybind;

    use super::*;

    fn action(name: &str, assigned_keys: &str, running: bool) -> AppAction {
        AppAction {
            app_id: "recorder".to_owned(),
            name: name.to_owned(),
            description: String::new(),
            preferred_keys: Keybind::NONE,
            wanted_keys: Keybind::NONE,
            assigned_keys: assigned_keys
                .parse()
                .unwrap_or_else(|error| panic!("`{assigned_keys}`: {error}")),
            running,
        }
    }

    #[test]
    fn a_stopped_action_stays_in_the_list_as_not_running() {
        let mut known = vec![action("toggle", "Super+Alt+R", true)];
        record_registered_actions(&mut known, []);
        assert_eq!(known, [action("toggle", "Super+Alt+R", false)]);
    }

    #[test]
    fn a_registered_action_replaces_its_old_row_and_a_new_one_is_added() {
        let mut known = vec![
            action("toggle", "Super+Alt+R", false),
            action("pause", "None", false),
        ];
        record_registered_actions(
            &mut known,
            [
                action("toggle", "Ctrl+F9", true),
                action("marker", "Super+Alt+M", true),
            ],
        );
        assert_eq!(
            known,
            [
                action("toggle", "Ctrl+F9", true),
                action("pause", "None", false),
                action("marker", "Super+Alt+M", true),
            ]
        );
    }
}

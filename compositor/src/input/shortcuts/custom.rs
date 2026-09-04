//! Config rows, turned into overrides on the default table.

use crownos_config::{Keybind, keybind::ParseKeybindError};
use thiserror::Error;

use config::Binding;

use crate::input::shortcuts::{
    action::{Action, ParseActionError},
    chord::Chord,
};

/// One `keybinds.custom_keybinds` row that made it through parsing.
#[derive(Debug, PartialEq)]
pub struct Override {
    pub chord: Chord,
    pub action: Action,
}

#[derive(Debug, Error)]
pub enum ParseBindingError {
    #[error("invalid shortcut: {0}")]
    Keys(#[from] ParseKeybindError),
    #[error("invalid action: {0}")]
    Action(#[from] ParseActionError),
}

impl TryFrom<&Binding> for Override {
    type Error = ParseBindingError;

    fn try_from(binding: &Binding) -> Result<Self, Self::Error> {
        Ok(Self {
            chord: binding.keys.parse::<Keybind>()?.into(),
            action: binding.action.parse()?,
        })
    }
}

/// Every row that parses, in file order.
///
/// A row that does not is logged and dropped: one typo must not take the rest
/// of the file with it, and least of all the defaults it was laid over.
pub fn overrides(custom: &[Binding]) -> impl Iterator<Item = Override> + '_ {
    custom.iter().filter_map(|binding| {
        Override::try_from(binding)
            .inspect_err(|err| {
                tracing::warn!(
                    %err,
                    keys = %binding.keys,
                    action = %binding.action,
                    "ignoring keybind"
                );
            })
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(keys: &str, action: &str) -> Binding {
        Binding {
            keys: keys.to_owned(),
            action: action.to_owned(),
        }
    }

    #[test]
    fn a_row_becomes_a_chord_and_an_action() {
        let parsed = Override::try_from(&row("Super+Shift+B", "spawn firefox"))
            .expect("a well-formed row parses");

        assert_eq!(
            parsed.chord,
            "Super+Shift+B".parse::<Keybind>().unwrap().into()
        );
        assert_eq!(parsed.action, Action::Spawn(vec!["firefox".to_owned()]));
    }

    #[test]
    fn errors_say_which_half_of_the_row_is_wrong() {
        let err = Override::try_from(&row("Supper+Q", "quit")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid shortcut: not a modifier or a key a shortcut may use: Supper"
        );

        let err = Override::try_from(&row("Super+Q", "frobnicate")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid action: unknown action `frobnicate`"
        );
    }

    #[test]
    fn a_bad_row_is_dropped_and_the_rest_survive() {
        let rows = [
            row("Supper+Q", "quit"),
            row("Super+Q", "frobnicate"),
            row("Super+E", "quit"),
        ];

        let kept: Vec<Override> = overrides(&rows).collect();

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].action, Action::Quit);
    }
}

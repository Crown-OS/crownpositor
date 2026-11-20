//! What a nested output says about itself in place of an EDID.
//!
//! `CROWN_NESTED_EDID="winit-1=BOE 0B45 2;winit-2=Dell Inc. U2720Q 7XK2"`
//! gives a viewport the make, model and serial of a real monitor, so its
//! settings are filed under that identity, and two viewports with the same
//! value are twins. The last word is the serial and the word before it the
//! model. A value that ends in a space has an empty serial.

use crownos_config::schema::output_identity;

const EDID_VARIABLE: &str = "CROWN_NESTED_EDID";
const MAKE: &str = "Crownpositor";
const MODEL: &str = "Winit";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedEdid {
    pub make: String,
    pub model: String,
    pub serial: Option<String>,
}

impl Default for NestedEdid {
    fn default() -> Self {
        Self {
            make: MAKE.to_owned(),
            model: MODEL.to_owned(),
            serial: None,
        }
    }
}

impl NestedEdid {
    fn parse(value: &str) -> Option<Self> {
        let (make_and_model, serial) = value.trim_start().rsplit_once(' ')?;
        let (make, model) = make_and_model.trim_end().rsplit_once(' ')?;
        let make = make.trim_end();
        if make.is_empty() || model.is_empty() {
            return None;
        }
        Some(Self {
            make: make.to_owned(),
            model: model.to_owned(),
            serial: (!serial.is_empty()).then(|| serial.to_owned()),
        })
    }

    /// The identity a real monitor with this EDID would get on `connector`.
    pub fn identity(&self, connector: &str) -> String {
        output_identity(connector, &self.make, &self.model, self.serial.as_deref())
    }
}

/// The EDID the environment gives the viewport `name`, else the default.
pub fn nested_edid(name: &str) -> NestedEdid {
    std::env::var(EDID_VARIABLE)
        .ok()
        .and_then(|value| edid_named(&value, name))
        .unwrap_or_default()
}

fn edid_named(value: &str, name: &str) -> Option<NestedEdid> {
    let (_, edid) = value
        .split(';')
        .filter_map(|entry| entry.split_once('='))
        .find(|(connector, _)| connector.trim() == name)?;
    let parsed = NestedEdid::parse(edid);
    if parsed.is_none() {
        tracing::warn!(
            output = name,
            edid,
            "{EDID_VARIABLE} needs MAKE MODEL SERIAL, ignoring it"
        );
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUE: &str = "winit-1=BOE 0B45 2;winit-2=Dell Inc. U2720Q 7XK2;winit-3=Acme Pro ";

    fn edid(make: &str, model: &str, serial: Option<&str>) -> NestedEdid {
        NestedEdid {
            make: make.to_owned(),
            model: model.to_owned(),
            serial: serial.map(str::to_owned),
        }
    }

    #[test]
    fn the_last_two_words_are_the_serial_and_the_model() {
        assert_eq!(
            edid_named(VALUE, "winit-1"),
            Some(edid("BOE", "0B45", Some("2")))
        );
        assert_eq!(
            edid_named(VALUE, "winit-2"),
            Some(edid("Dell Inc.", "U2720Q", Some("7XK2")))
        );
    }

    #[test]
    fn a_trailing_space_is_an_empty_serial() {
        assert_eq!(
            edid_named(VALUE, "winit-3"),
            Some(edid("Acme", "Pro", None))
        );
    }

    #[test]
    fn the_identity_matches_a_real_monitor_with_the_same_edid() {
        let dell = edid("Dell Inc.", "U2720Q", Some("7XK2"));
        assert_eq!(dell.identity("winit-2"), "Dell Inc. U2720Q 7XK2");
        assert_eq!(edid("Acme", "Pro", None).identity("winit-3"), "winit-3");
        assert_eq!(NestedEdid::default().identity("winit-1"), "winit-1");
    }

    #[test]
    fn an_unnamed_or_short_value_gives_nothing() {
        assert_eq!(edid_named(VALUE, "winit-4"), None);
        assert_eq!(edid_named("winit-1=0B45 2", "winit-1"), None);
        assert_eq!(edid_named("winit-1=BOE", "winit-1"), None);
    }
}

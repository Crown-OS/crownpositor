//! A head's entry in `compositor.ron`. A head named `IFACE-N@ADDRESS` was
//! named `IFACE-N` before its GPU address was added, so when nothing is filed
//! under its name or identity, the entry under the old name is its own.

use config::{Compositor, Config, OutputSetting};

use crate::utils::head_name::{pci_suffix, unqualified};

/// The setting for the head `connector` with `identity`.
pub fn output_setting<'a>(
    config: &'a Config,
    connector: &str,
    identity: &str,
) -> Option<&'a OutputSetting> {
    config
        .output_setting(connector, identity)
        .or_else(|| legacy_name(connector).and_then(|legacy| config.output_setting(legacy, legacy)))
}

/// The entry [`output_setting`] reads, to edit. An entry found only under
/// the old name is copied to a new entry under `identity`, so the head on
/// the primary GPU that now goes by the old name keeps that entry to itself.
pub fn output_entry<'a>(
    compositor: &'a mut Compositor,
    connector: &str,
    identity: &str,
) -> &'a mut OutputSetting {
    if compositor.output(connector, identity).is_none()
        && let Some(inherited) = legacy_name(connector)
            .and_then(|legacy| compositor.output(legacy, legacy))
            .cloned()
    {
        compositor.outputs.push(OutputSetting {
            name: identity.to_owned(),
            ..inherited
        });
    }
    compositor.output_entry(connector, identity)
}

fn legacy_name(connector: &str) -> Option<&str> {
    pci_suffix(connector).map(|_| unqualified(connector))
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUALIFIED: &str = "HDMI-A-1@0000:01:00.0";
    const IDENTITY: &str = "Acme X 1";

    fn entry(name: &str, bar: bool) -> OutputSetting {
        OutputSetting {
            name: name.to_owned(),
            bar: Some(bar),
            scale: Some(1.5),
            ..OutputSetting::default()
        }
    }

    fn config(outputs: Vec<OutputSetting>) -> Config {
        let mut config = Config::default();
        config.compositor.outputs = outputs;
        config
    }

    fn config_without(name: &str, mut config: Config) -> Config {
        config.compositor.outputs.retain(|entry| entry.name != name);
        config
    }

    fn name_of(setting: Option<&OutputSetting>) -> Option<&str> {
        setting.map(|setting| setting.name.as_str())
    }

    #[test]
    fn a_qualified_head_falls_back_to_the_entry_under_its_old_name() {
        let config = config(vec![entry("HDMI-A-1", false)]);
        assert_eq!(
            name_of(output_setting(&config, QUALIFIED, IDENTITY)),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn its_own_name_or_identity_wins_over_the_old_name() {
        let config = config(vec![
            entry("HDMI-A-1", false),
            entry(IDENTITY, true),
            entry(QUALIFIED, true),
        ]);
        assert_eq!(
            name_of(output_setting(&config, QUALIFIED, IDENTITY)),
            Some(QUALIFIED)
        );
        let config = config_without(QUALIFIED, config);
        assert_eq!(
            name_of(output_setting(&config, QUALIFIED, IDENTITY)),
            Some(IDENTITY)
        );
    }

    #[test]
    fn a_plain_head_never_reads_another_entry() {
        let config = config(vec![entry("HDMI-A-1", false)]);
        assert_eq!(output_setting(&config, "HDMI-A-2", IDENTITY), None);
    }

    #[test]
    fn editing_through_the_old_name_copies_it_under_the_identity() {
        let mut compositor = config(vec![entry("HDMI-A-1", false)]).compositor;
        output_entry(&mut compositor, QUALIFIED, IDENTITY).scale = Some(2.0);
        assert_eq!(compositor.outputs.len(), 2);
        assert_eq!(compositor.outputs[0].scale, Some(1.5));
        assert_eq!(compositor.outputs[1].name, IDENTITY);
        assert_eq!(compositor.outputs[1].bar, Some(false));
        assert_eq!(compositor.outputs[1].scale, Some(2.0));
    }
}

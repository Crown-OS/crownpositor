//! The arrangement saved for the set of monitors plugged in now, and what it
//! says about where each monitor goes and whether it is lit.

use config::{OutputLayout, OutputSetting};

use crate::utils::output_setting::output_setting;
use crate::{
    shell::{Shell, output_kind::NestedPersistence},
    state::{BackendState, State},
};

/// The two names a monitor is filed under in `compositor.ron`.
#[derive(Debug, Clone, Copy)]
pub struct OutputNames<'a> {
    pub identity: &'a str,
    pub connector: &'a str,
}

impl OutputNames<'_> {
    fn named_in(&self, names: &[String]) -> bool {
        names
            .iter()
            .any(|name| name == self.identity || name == self.connector)
    }
}

/// The identities of the monitors whose arrangement is saved: the persisted
/// monitors, then the persisted heads that are switched off.
pub fn output_set(shell: &Shell, nested: NestedPersistence) -> Vec<String> {
    let lit = shell
        .persisted_monitors(nested)
        .map(|monitor| monitor.config().connector.as_str().to_owned());
    let switched_off = shell
        .offline_heads()
        .iter()
        .filter(|head| head.kind.is_persisted(nested))
        .map(|head| head.identity.clone());
    lit.chain(switched_off).collect()
}

/// The first saved arrangement made for exactly this set of monitors.
pub fn matched_layout<'a>(layouts: &'a [OutputLayout], set: &[String]) -> Option<&'a OutputLayout> {
    layouts.iter().find(|layout| layout.matches(set))
}

/// Where the config pins a monitor: its place in the matched arrangement,
/// else its own `position`. `None` packs it right of the others, which is
/// also what a matched arrangement that leaves the monitor out asks for.
pub fn configured_position(
    layout: Option<&OutputLayout>,
    names: OutputNames,
    setting: Option<&OutputSetting>,
) -> Option<(i32, i32)> {
    match layout {
        Some(layout) => layout
            .position_of(names.identity)
            .or_else(|| layout.position_of(names.connector)),
        None => setting.and_then(|setting| setting.position),
    }
}

/// Whether a head is lit: not switched off by its own setting, by the
/// matched arrangement, or by `suppressed` (a closed lid).
pub fn should_enable(
    setting: Option<&OutputSetting>,
    layout: Option<&OutputLayout>,
    names: OutputNames,
    suppressed: &[String],
) -> bool {
    setting.and_then(|setting| setting.enabled) != Some(false)
        && !layout.is_some_and(|layout| names.named_in(&layout.disabled))
        && !names.named_in(suppressed)
}

/// The indices of the heads to light: those wanted, or the first when none
/// is wanted and nothing is lit, so the session never starts dark.
pub fn heads_to_light(wanted: &[bool], anything_lit: bool) -> Vec<usize> {
    let lit: Vec<usize> = wanted
        .iter()
        .enumerate()
        .filter_map(|(index, wanted)| wanted.then_some(index))
        .collect();
    match (lit.is_empty(), anything_lit, wanted.is_empty()) {
        (true, false, false) => vec![0],
        _ => lit,
    }
}

impl State {
    /// The identities of the monitors whose arrangement is saved, lit or
    /// switched off.
    pub fn output_set(&self) -> Vec<String> {
        output_set(&self.shell, NestedPersistence::from_environment())
    }

    pub fn matched_output_layout(&self) -> Option<OutputLayout> {
        matched_layout(
            &self.config.current.compositor.output_layouts,
            &self.output_set(),
        )
        .cloned()
    }

    /// Whether the head named `connector` should be lit now.
    pub fn should_enable_head(&self, connector: &str) -> bool {
        let identity = self
            .shell
            .offline_heads()
            .iter()
            .find(|head| head.connector == connector)
            .map_or(connector, |head| head.identity.as_str());
        let names = OutputNames {
            identity,
            connector,
        };
        should_enable(
            output_setting(&self.config.current, connector, identity),
            self.matched_output_layout().as_ref(),
            names,
            self.clamshell.suppressed(),
        )
    }

    /// Pushes the per-output config, placed by the matched arrangement, onto
    /// every monitor, and sets each mode that changed on the hardware too.
    pub fn apply_output_settings(&mut self) {
        let layout = self.matched_output_layout();
        let changed_modes = self
            .shell
            .apply_output_settings(&self.config.current, layout.as_ref());
        for (output, mode) in changed_modes {
            if let Err(err) = self.backend.set_output_mode(&output, mode) {
                tracing::error!(%err, output = output.name(), "failed to set the configured mode");
            }
        }
    }

    /// After a monitor arrives or leaves: switches off what the arrangement
    /// for the new set has off, then places every monitor by it.
    pub fn reconcile_output_layout(&mut self) {
        self.sync_offline_heads();
        let nested = NestedPersistence::from_environment();
        let switched_off: Vec<String> = match self.matched_output_layout() {
            Some(layout) => self
                .shell
                .persisted_monitors(nested)
                .map(|monitor| monitor.config())
                .filter(|config| {
                    OutputNames {
                        identity: config.connector.as_str(),
                        connector: &config.name,
                    }
                    .named_in(&layout.disabled)
                })
                .map(|config| config.name.clone())
                .collect(),
            None => Vec::new(),
        };
        for name in switched_off {
            if self.shell.shown_monitors().count() < 2 {
                break;
            }
            if let Err(err) = BackendState::set_head_enabled(self, &name, false) {
                tracing::error!(%err, output = name, "failed to switch a monitor off");
            }
        }
        self.apply_output_settings();
    }

    /// Which of `connectors` to light, in their order: those that should be,
    /// or the first when none should and nothing is lit yet.
    pub fn connectors_to_light<'a>(&self, connectors: &[&'a str]) -> Vec<&'a str> {
        let wanted: Vec<bool> = connectors
            .iter()
            .map(|connector| self.should_enable_head(connector))
            .collect();
        let anything_lit = self.shell.shown_monitors().next().is_some();
        heads_to_light(&wanted, anything_lit)
            .into_iter()
            .filter_map(|index| connectors.get(index).copied())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCKED: [&str; 2] = ["eDP-1", "Dell U2720Q A1"];

    fn names<'a>(identity: &'a str, connector: &'a str) -> OutputNames<'a> {
        OutputNames {
            identity,
            connector,
        }
    }

    fn layout(heads: &[&str], positions: &[(&str, i32, i32)], disabled: &[&str]) -> OutputLayout {
        OutputLayout {
            heads: heads.iter().map(|head| (*head).to_owned()).collect(),
            positions: positions
                .iter()
                .map(|(name, x, y)| ((*name).to_owned(), *x, *y))
                .collect(),
            disabled: disabled.iter().map(|head| (*head).to_owned()).collect(),
        }
    }

    fn set(heads: &[&str]) -> Vec<String> {
        heads.iter().map(|head| (*head).to_owned()).collect()
    }

    fn positioned(position: (i32, i32)) -> OutputSetting {
        OutputSetting {
            position: Some(position),
            ..OutputSetting::default()
        }
    }

    #[test]
    fn the_arrangement_for_the_exact_set_is_matched_in_any_order() {
        let layouts = [layout(&["eDP-1"], &[], &[]), layout(&DOCKED, &[], &[])];
        let reversed = set(&[DOCKED[1], DOCKED[0]]);
        assert_eq!(matched_layout(&layouts, &reversed), Some(&layouts[1]));
        assert_eq!(matched_layout(&layouts, &set(&["DP-9"])), None);
    }

    #[test]
    fn the_matched_arrangement_places_a_monitor_before_its_own_position() {
        let docked = layout(&DOCKED, &[("Dell U2720Q A1", 1920, 0)], &[]);
        let setting = positioned((5, 5));
        let dell = names("Dell U2720Q A1", "DP-1");
        assert_eq!(
            configured_position(Some(&docked), dell, Some(&setting)),
            Some((1920, 0))
        );
        assert_eq!(
            configured_position(None, dell, Some(&setting)),
            Some((5, 5))
        );
        assert_eq!(configured_position(None, dell, None), None);
    }

    #[test]
    fn a_matched_arrangement_that_leaves_a_monitor_out_packs_it() {
        let docked = layout(&DOCKED, &[("eDP-1", 0, 0)], &[]);
        let setting = positioned((5, 5));
        assert_eq!(
            configured_position(
                Some(&docked),
                names("Dell U2720Q A1", "DP-1"),
                Some(&setting)
            ),
            None
        );
    }

    #[test]
    fn an_arrangement_may_name_a_monitor_by_its_connector() {
        let docked = layout(&["eDP-1", "DP-1"], &[("DP-1", 1920, 0)], &[]);
        assert_eq!(
            configured_position(Some(&docked), names("Dell U2720Q A1", "DP-1"), None),
            Some((1920, 0))
        );
    }

    #[test]
    fn the_first_head_is_lit_when_every_head_is_turned_off_and_nothing_is_lit() {
        assert_eq!(heads_to_light(&[false, true, true], false), [1, 2]);
        assert_eq!(heads_to_light(&[false, false], false), [0]);
        assert!(heads_to_light(&[false, false], true).is_empty());
        assert!(heads_to_light(&[], false).is_empty());
    }

    #[test]
    fn a_head_is_lit_unless_its_setting_or_the_arrangement_turns_it_off() {
        let panel = names("eDP-1", "eDP-1");
        let off = OutputSetting {
            enabled: Some(false),
            ..OutputSetting::default()
        };
        let lid_closed_layout = layout(&DOCKED, &[], &["eDP-1"]);

        assert!(should_enable(None, None, panel, &[]));
        assert!(!should_enable(Some(&off), None, panel, &[]));
        assert!(!should_enable(None, Some(&lid_closed_layout), panel, &[]));
        assert!(!should_enable(None, None, panel, &set(&["eDP-1"])));
        assert!(should_enable(
            None,
            Some(&lid_closed_layout),
            names("Dell U2720Q A1", "DP-1"),
            &[]
        ));
    }
}

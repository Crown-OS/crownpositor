//! Clamshell mode. With the lid closed and another monitor on, the laptop's
//! own panel is switched off. It comes back when the lid opens, or when it
//! would be the only monitor left. The panel switched off here is never
//! saved as switched off.

use crate::{
    shell::output_kind::OutputKind,
    state::{BackendState, State},
    utils::panel::is_internal,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Lid {
    #[default]
    Open,
    Closed,
}

#[derive(Debug, Default)]
pub struct Clamshell {
    lid: Lid,
    /// The panel switched off because the lid is closed.
    suppressed: Option<String>,
}

impl Clamshell {
    /// The heads switched off because the lid is closed: none or the panel.
    pub fn suppressed(&self) -> &[String] {
        self.suppressed.as_slice()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelChange {
    SwitchOff(String),
    SwitchOn(String),
    Keep,
}

/// What to do with the panel: `lit_panel` is the built-in panel when it is
/// on, `others_on` the number of other monitors that are on.
pub fn decide(
    lid: Lid,
    suppressed: Option<&str>,
    lit_panel: Option<&str>,
    others_on: usize,
) -> PanelChange {
    match (lid, suppressed, lit_panel) {
        (Lid::Closed, None, Some(panel)) if others_on > 0 => {
            PanelChange::SwitchOff(panel.to_owned())
        }
        (Lid::Open, Some(panel), _) => PanelChange::SwitchOn(panel.to_owned()),
        (Lid::Closed, Some(panel), _) if others_on == 0 => PanelChange::SwitchOn(panel.to_owned()),
        _ => PanelChange::Keep,
    }
}

impl State {
    pub fn set_lid(&mut self, lid: Lid) {
        if self.clamshell.lid != lid {
            tracing::info!(?lid, "lid switch");
        }
        self.clamshell.lid = lid;
        self.check_clamshell();
    }

    /// Switches the panel off or on for the lid and the monitors there are
    /// now. A virtual output, such as a remote tablet, does not count as a
    /// monitor the panel can give way to.
    pub fn check_clamshell(&mut self) {
        let lit_panel = self
            .shell
            .shown_monitors()
            .map(|monitor| monitor.config().name.as_str())
            .find(|name| is_internal(name));
        let others_on = self
            .shell
            .shown_monitors()
            .map(|monitor| monitor.config())
            .filter(|config| config.kind != OutputKind::Virtual && !is_internal(&config.name))
            .count();
        let change = decide(
            self.clamshell.lid,
            self.clamshell.suppressed.as_deref(),
            lit_panel,
            others_on,
        );
        match change {
            PanelChange::SwitchOff(panel) => {
                self.clamshell.suppressed = Some(panel.clone());
                if !self.switch_panel(&panel, false) {
                    self.clamshell.suppressed = None;
                }
            }
            PanelChange::SwitchOn(panel) => {
                self.clamshell.suppressed = None;
                self.switch_panel(&panel, true);
            }
            PanelChange::Keep => {}
        }
    }

    fn switch_panel(&mut self, panel: &str, enabled: bool) -> bool {
        tracing::info!(output = panel, enabled, "clamshell switches the panel");
        match BackendState::set_head_enabled(self, panel, enabled) {
            Ok(found) => found,
            Err(err) => {
                tracing::error!(%err, output = panel, enabled, "failed to switch the panel");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: &str = "eDP-1";

    fn off() -> PanelChange {
        PanelChange::SwitchOff(PANEL.to_owned())
    }

    fn on() -> PanelChange {
        PanelChange::SwitchOn(PANEL.to_owned())
    }

    #[test]
    fn closing_the_lid_with_another_monitor_on_switches_the_panel_off() {
        assert_eq!(decide(Lid::Closed, None, Some(PANEL), 1), off());
    }

    #[test]
    fn closing_the_lid_with_no_other_monitor_keeps_the_panel() {
        assert_eq!(decide(Lid::Closed, None, Some(PANEL), 0), PanelChange::Keep);
    }

    #[test]
    fn opening_the_lid_brings_the_panel_back() {
        assert_eq!(decide(Lid::Open, Some(PANEL), None, 1), on());
    }

    #[test]
    fn the_panel_comes_back_when_the_last_other_monitor_goes() {
        assert_eq!(decide(Lid::Closed, Some(PANEL), None, 0), on());
        assert_eq!(decide(Lid::Closed, Some(PANEL), None, 1), PanelChange::Keep);
    }

    #[test]
    fn a_panel_the_config_switched_off_is_left_alone() {
        assert_eq!(decide(Lid::Closed, None, None, 1), PanelChange::Keep);
        assert_eq!(decide(Lid::Open, None, None, 1), PanelChange::Keep);
    }

    #[test]
    fn an_open_lid_changes_nothing_while_the_panel_is_on() {
        assert_eq!(decide(Lid::Open, None, Some(PANEL), 2), PanelChange::Keep);
    }
}

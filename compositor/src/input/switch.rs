//! The lid switch. Its state when a switch device appears is read from
//! ACPI: libinput is not known to send an event for a lid that is closed
//! already.

use std::{fs, path::Path};

use smithay::backend::input::{
    Device, DeviceCapability, InputBackend, Switch, SwitchState, SwitchToggleEvent,
};

use crate::state::{State, clamshell::Lid};

const ACPI_LIDS: &str = "/proc/acpi/button/lid";

impl State {
    pub(super) fn on_switch_toggle<I: InputBackend>(&mut self, event: &I::SwitchToggleEvent) {
        if event.switch() != Some(Switch::Lid) {
            return;
        }
        self.set_lid(match event.state() {
            SwitchState::On => Lid::Closed,
            SwitchState::Off => Lid::Open,
        });
    }

    /// Reads the lid once a switch device appears, at start and again after
    /// libinput adds its devices back on resume.
    pub(super) fn on_switch_added<I: InputBackend>(&mut self, device: &I::Device) {
        if !device.has_capability(DeviceCapability::Switch) {
            return;
        }
        if let Some(lid) = acpi_lid(Path::new(ACPI_LIDS)) {
            self.set_lid(lid);
        }
    }
}

/// The first lid ACPI knows about, from its `state` file.
fn acpi_lid(lids: &Path) -> Option<Lid> {
    fs::read_dir(lids)
        .ok()?
        .flatten()
        .find_map(|lid| fs::read_to_string(lid.path().join("state")).ok())
        .and_then(|state| parse_lid_state(&state))
}

/// `"state:      closed"` or `"state:      open"`.
fn parse_lid_state(text: &str) -> Option<Lid> {
    match text.split_whitespace().last()? {
        "closed" => Some(Lid::Closed),
        "open" => Some(Lid::Open),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_acpi_state_line_gives_the_lid() {
        assert_eq!(parse_lid_state("state:      closed\n"), Some(Lid::Closed));
        assert_eq!(parse_lid_state("state:      open\n"), Some(Lid::Open));
        assert_eq!(parse_lid_state("state:      unknown\n"), None);
        assert_eq!(parse_lid_state(""), None);
    }

    #[test]
    fn a_machine_without_a_lid_reads_nothing() {
        assert_eq!(acpi_lid(Path::new("/nonexistent/lid")), None);
    }
}

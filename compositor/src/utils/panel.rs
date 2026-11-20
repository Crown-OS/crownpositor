//! Telling a laptop's own panel from a monitor plugged into it, by the
//! connector the kernel names it after.

use crate::utils::head_name::unqualified;

const INTERNAL_INTERFACES: [&str; 3] = ["eDP", "LVDS", "DSI"];

/// Whether the head (`"eDP-1"`, `"DP-2"`, `"eDP-1@0000:01:00.0"`) is built
/// into the machine.
pub fn is_internal(head: &str) -> bool {
    let connector = unqualified(head);
    let interface = connector
        .rsplit_once('-')
        .map_or(connector, |(interface, _)| interface);
    INTERNAL_INTERFACES.contains(&interface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laptop_panels_are_internal() {
        for name in [
            "eDP-1",
            "eDP-2",
            "LVDS-1",
            "DSI-1",
            "eDP-1@0000:01:00.0",
            "eDP-1@platform-fd5a0000.gpu",
        ] {
            assert!(is_internal(name), "{name} is a built-in panel");
        }
    }

    #[test]
    fn plugged_in_monitors_are_not_internal() {
        for name in [
            "DP-1",
            "HDMI-A-1",
            "DVI-D-1",
            "winit-1",
            "VIRTUAL-1",
            "HEADLESS-1",
            "DP-1@platform-fd5a0000.gpu",
        ] {
            assert!(!is_internal(name), "{name} is not a built-in panel");
        }
    }
}

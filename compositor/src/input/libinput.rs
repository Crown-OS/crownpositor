//! Per-device libinput defaults.
//!
//! libinput's own defaults are conservative and inherited from X11 — most
//! notably tap-to-click off, on the reasoning that a touchpad with a physical
//! button under it can always be clicked. Every laptop this compositor targets
//! expects the tap, so the default is flipped here rather than handed to a
//! setting: there is no reasonable machine on which the other answer is right.
//!
//! Applied on `DeviceAdded`, which is also where a hotplugged or VT-resumed
//! device comes back, so a touchpad plugged in mid-session is configured the
//! same as one present at boot.

use smithay::reexports::input::Device;

/// Applies the compositor's defaults to a device that just appeared.
///
/// Only touchpads are touched. `config_tap_finger_count` is libinput's own
/// answer to "can this device tap at all" — zero for a mouse, a trackpoint or a
/// keyboard, and zero as well for the rare touchpad whose driver cannot report
/// taps — which makes it a sounder test than matching on device names.
pub fn apply_defaults(device: &mut Device) {
    if device.config_tap_finger_count() == 0 {
        return;
    }

    // Tap to click, and with it libinput's default finger mapping: one finger
    // is a left click, two a right click, three a middle click.
    if let Err(err) = device.config_tap_set_enabled(true) {
        tracing::warn!(
            ?err,
            device = device.name(),
            "touchpad refused tap-to-click"
        );
        return;
    }

    // Tap-and-drag: a tap immediately followed by a press and a move drags
    // rather than clicking twice. It is the half of tap-to-click that selects
    // text and moves windows, and libinput leaves it on by default — set
    // explicitly so the pair is legible as one decision.
    if let Err(err) = device.config_tap_set_drag_enabled(true) {
        tracing::warn!(
            ?err,
            device = device.name(),
            "touchpad refused tap-and-drag"
        );
    }

    tracing::info!(device = device.name(), "tap-to-click enabled");
}

//! Connector properties DRM exposes but smithay does not wrap.
//!
//! `DrmSurface` covers the properties it needs to drive a modeset; everything
//! a compositor wants to *know* about a connector — its EDID, whether it can
//! do adaptive sync, how many bits per channel it will accept — is reachable
//! only by walking the property list by name. That walk is the same every
//! time, so it lives here once.

use std::ops::RangeInclusive;

use smithay::reexports::drm::control::{
    Device as ControlDevice, connector,
    property::{self, Value, ValueType},
};

use crate::utils::edid::{self, EdidInfo};

/// A property's type and current raw value, found by name.
///
/// The type comes back owned rather than as a `Value`, because
/// `ValueType::convert_value` borrows from it for enum properties — so the
/// caller has to hold it while the converted value is alive.
fn find(
    device: &impl ControlDevice,
    connector: connector::Handle,
    name: &str,
) -> Option<(ValueType, property::RawValue)> {
    let properties = device.get_properties(connector).ok()?;
    properties.iter().find_map(|(handle, raw)| {
        let info = device.get_property(*handle).ok()?;
        (info.name().to_str() == Ok(name)).then(|| (info.value_type(), *raw))
    })
}

/// Reads a property that is either a boolean or an unsigned range, which is
/// how drivers disagree about spelling a flag.
fn flag(device: &impl ControlDevice, connector: connector::Handle, name: &str) -> bool {
    let Some((kind, raw)) = find(device, connector, name) else {
        return false;
    };
    match kind.convert_value(raw) {
        Value::Boolean(set) => set,
        Value::UnsignedRange(value) => value != 0,
        _ => false,
    }
}

/// The connector's EDID, parsed.
///
/// `None` covers every failure the same way — no property, an unreadable blob,
/// or bytes that are not an EDID — because the caller's response to all three
/// is identical: fall back to the connector name for identity.
pub fn edid(device: &impl ControlDevice, connector: connector::Handle) -> Option<EdidInfo> {
    let (kind, raw) = find(device, connector, "EDID")?;
    let blob = kind.convert_value(raw).as_blob()?;
    let bytes = device.get_property_blob(blob).ok()?;
    edid::parse(&bytes)
}

/// The `max bpc` property: what it is set to now, and what it will accept.
///
/// Driving 10 bits per channel is a precondition for HDR, and the range is
/// what makes a configured `max_bpc` testable before it is applied.
pub fn max_bpc(
    device: &impl ControlDevice,
    connector: connector::Handle,
) -> Option<(u64, RangeInclusive<u64>)> {
    let (kind, raw) = find(device, connector, "max bpc")?;
    let ValueType::UnsignedRange(low, high) = kind else {
        return None;
    };
    let current = kind.convert_value(raw).as_unsigned_range()?;
    Some((current, low..=high))
}

/// Whether the connector can do adaptive sync at all.
///
/// Read from the connector rather than from `DrmSurface::vrr_supported`
/// because a *disabled* head has no surface, and the head list has to report
/// its capability anyway.
pub fn vrr_capable(device: &impl ControlDevice, connector: connector::Handle) -> bool {
    flag(device, connector, "vrr_capable")
}

/// Whether the kernel marks this connector as not part of the desktop.
///
/// VR headsets set this. Such a connector must never be lit as an output and
/// must stay out of the CRTC pool, so it can be leased to a client instead.
pub fn non_desktop(device: &impl ControlDevice, connector: connector::Handle) -> bool {
    flag(device, connector, "non-desktop")
}

//! A built-in panel's backlight.
//!
//! sysfs is world-readable, but its `brightness` file is root's. logind's
//! `SetBrightness` lets the active session write it with no udev rule and no
//! polkit prompt.

use zbus::{Connection, proxy};

use super::{device::Sysfs, error::DeviceError};

const SUBSYSTEM: &str = "backlight";

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1/session/auto"
)]
trait Session {
    fn set_brightness(&self, subsystem: &str, name: &str, brightness: u32) -> zbus::Result<()>;
}

pub fn read(sysfs: &Sysfs, name: &str) -> Result<u32, DeviceError> {
    sysfs
        .backlight_value(name, "brightness")
        .ok_or_else(|| DeviceError::Unreadable(name.to_owned()))
}

pub async fn write(bus: &Connection, name: &str, raw: u32) -> Result<(), DeviceError> {
    SessionProxy::new(bus)
        .await?
        .set_brightness(SUBSYSTEM, name, raw)
        .await?;
    Ok(())
}

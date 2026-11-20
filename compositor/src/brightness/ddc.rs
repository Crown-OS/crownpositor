//! An external monitor's own brightness over DDC/CI.
//!
//! Every call blocks for tens of milliseconds, so the worker runs them on the
//! blocking pool, one at a time: writes in parallel on one i2c bus are
//! slower, not faster.

use std::fmt::Display;

use ddc::Ddc;
use ddc_i2c::I2cDeviceDdc;

use super::error::DeviceError;

/// The MCCS feature every monitor implements for luminance.
const LUMINANCE: u8 = 0x10;

pub fn open(bus: u32) -> Result<I2cDeviceDdc, DeviceError> {
    ddc_i2c::from_i2c_device(format!("/dev/i2c-{bus}")).map_err(|error| failed(bus, error))
}

/// The luminance and the top of the monitor's scale.
pub fn read(monitor: &mut I2cDeviceDdc, bus: u32) -> Result<(u32, u32), DeviceError> {
    let value = monitor
        .get_vcp_feature(LUMINANCE)
        .map_err(|error| failed(bus, error))?;
    Ok((u32::from(value.value()), u32::from(value.maximum())))
}

pub fn write(monitor: &mut I2cDeviceDdc, bus: u32, raw: u32) -> Result<(), DeviceError> {
    let raw = u16::try_from(raw).unwrap_or(u16::MAX);
    monitor
        .set_vcp_feature(LUMINANCE, raw)
        .map_err(|error| failed(bus, error))
}

fn failed(bus: u32, error: impl Display) -> DeviceError {
    DeviceError::Ddc {
        bus,
        message: error.to_string(),
    }
}

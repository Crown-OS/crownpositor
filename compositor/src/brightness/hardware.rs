//! The real devices behind the worker: sysfs and logind for backlights,
//! `/dev/i2c-N` for DDC/CI. Every i2c call runs on the blocking pool.

use std::collections::HashMap;

use ddc_i2c::I2cDeviceDdc;
use zbus::Connection;

use super::{
    backlight, ddc,
    device::{Device, Sysfs},
    error::DeviceError,
    worker::{Hardware, Job, Operation, Outcome},
};

#[derive(Default)]
pub struct SystemHardware {
    sysfs: Sysfs,
    logind: Option<Connection>,
    /// Open buses, kept because each remembers when it may be spoken to next.
    monitors: HashMap<u32, I2cDeviceDdc>,
}

impl Hardware for SystemHardware {
    async fn run(&mut self, job: Job) -> Option<Outcome> {
        match job {
            Job::Probe(target) => {
                let output = target.output;
                let sysfs = self.sysfs.clone();
                let found = tokio::task::spawn_blocking(move || {
                    sysfs.probe(&target, |bus| {
                        let mut monitor = ddc::open(bus).ok()?;
                        ddc::read(&mut monitor, bus).ok()
                    })
                })
                .await
                .ok()?;
                Some(Outcome::Probed { output, found })
            }
            Job::Write {
                output,
                device,
                raw,
            } => self
                .write(&device, raw)
                .await
                .err()
                .map(|error| Outcome::Failed {
                    output,
                    during: Operation::Write,
                    error,
                }),
            Job::Read {
                output,
                sequence,
                device,
            } => Some(match self.read(&device).await {
                Ok(raw) => Outcome::Level {
                    output,
                    sequence,
                    raw,
                },
                Err(error) => Outcome::Failed {
                    output,
                    during: Operation::Read,
                    error,
                },
            }),
        }
    }
}

impl SystemHardware {
    async fn write(&mut self, device: &Device, raw: u32) -> Result<(), DeviceError> {
        match device {
            Device::Backlight { name, .. } => {
                let bus = self.logind().await?;
                backlight::write(&bus, name, raw).await
            }
            Device::Ddc { bus, .. } => {
                let bus = *bus;
                self.on_monitor(bus, move |monitor| ddc::write(monitor, bus, raw))
                    .await
            }
            Device::Software | Device::None => Err(DeviceError::NoHardware),
        }
    }

    async fn read(&mut self, device: &Device) -> Result<u32, DeviceError> {
        match device {
            Device::Backlight { name, .. } => backlight::read(&self.sysfs, name),
            Device::Ddc { bus, .. } => {
                let bus = *bus;
                self.on_monitor(bus, move |monitor| ddc::read(monitor, bus))
                    .await
                    .map(|(raw, _)| raw)
            }
            Device::Software | Device::None => Err(DeviceError::NoHardware),
        }
    }

    async fn logind(&mut self) -> Result<Connection, DeviceError> {
        if let Some(bus) = &self.logind {
            return Ok(bus.clone());
        }
        let bus = Connection::system().await?;
        self.logind = Some(bus.clone());
        Ok(bus)
    }

    /// Runs `call` on the blocking pool with the bus open, and keeps the bus
    /// open for the next call.
    async fn on_monitor<T: Send + 'static>(
        &mut self,
        bus: u32,
        call: impl FnOnce(&mut I2cDeviceDdc) -> Result<T, DeviceError> + Send + 'static,
    ) -> Result<T, DeviceError> {
        let held = self.monitors.remove(&bus);
        let (monitor, result) = tokio::task::spawn_blocking(move || {
            let mut monitor = match held {
                Some(monitor) => monitor,
                None => match ddc::open(bus) {
                    Ok(monitor) => monitor,
                    Err(error) => return (None, Err(error)),
                },
            };
            let result = call(&mut monitor);
            (Some(monitor), result)
        })
        .await
        .map_err(|_| DeviceError::Interrupted)?;
        if let Some(monitor) = monitor {
            self.monitors.insert(bus, monitor);
        }
        result
    }
}

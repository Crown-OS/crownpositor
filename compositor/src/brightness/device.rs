//! Which device holds the brightness of a connector, found through sysfs.
//!
//! A built-in panel uses its backlight: the one under its connector, else the
//! machine's own (firmware first, then platform, then raw). An external
//! monitor uses DDC/CI on the connector's DDC bus, else on its DP AUX bus, and
//! only if the monitor answers for luminance. Anything else is dimmed in the
//! gamma ramp when the output has one.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{curve::Curve, level::Control};
use crate::utils::{OutputId, panel::is_internal};

const BACKLIGHT_TYPES: [&str; 3] = ["firmware", "platform", "raw"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Device {
    /// A `/sys/class/backlight` device, written through logind.
    Backlight {
        name: String,
        max: u32,
    },
    /// A monitor on `/dev/i2c-<bus>`.
    Ddc {
        bus: u32,
        max: u32,
    },
    Software,
    None,
}

impl Device {
    pub fn control(&self) -> Control {
        match self {
            Self::Backlight { .. } => Control::Backlight,
            Self::Ddc { .. } => Control::Ddc,
            Self::Software => Control::Software,
            Self::None => Control::None,
        }
    }

    /// The curve and the top of the device's own scale, for a device the
    /// compositor writes to.
    pub fn scale(&self) -> Option<(Curve, u32)> {
        match self {
            Self::Backlight { max, .. } => Some((Curve::Perceptual, *max)),
            Self::Ddc { max, .. } => Some((Curve::Linear, *max)),
            Self::Software | Self::None => None,
        }
    }
}

/// A DRM card by its character device number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardNode {
    pub major: u32,
    pub minor: u32,
}

/// What the probe needs to know about one output.
#[derive(Debug, Clone)]
pub struct ProbeTarget {
    pub output: OutputId,
    pub card: CardNode,
    pub connector: String,
    pub has_gamma: bool,
}

/// A device and its value at the time it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub device: Device,
    pub raw: Option<u32>,
}

/// The sysfs tree, rooted elsewhere in tests.
#[derive(Debug, Clone)]
pub struct Sysfs {
    root: PathBuf,
}

impl Default for Sysfs {
    fn default() -> Self {
        Self::at("/sys")
    }
}

impl Sysfs {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Finds the device for `target`. `read_ddc` reads luminance from a bus
    /// as `(value, max)`, and fails for a monitor that does not answer.
    pub fn probe(
        &self,
        target: &ProbeTarget,
        mut read_ddc: impl FnMut(u32) -> Option<(u32, u32)>,
    ) -> Found {
        let connector = self
            .card_name(target.card)
            .map(|card| self.connector_directory(&card, &target.connector));
        let hardware = connector.and_then(|directory| {
            if is_internal(&target.connector) {
                let name = self
                    .backlight_under(&directory)
                    .or_else(|| self.machine_backlight())?;
                let max = self.backlight_value(&name, "max_brightness")?;
                let raw = self.backlight_value(&name, "brightness");
                Some(Found {
                    device: Device::Backlight { name, max },
                    raw,
                })
            } else {
                self.ddc_buses(&directory).into_iter().find_map(|bus| {
                    let (raw, max) = read_ddc(bus).filter(|(_, max)| *max > 0)?;
                    Some(Found {
                        device: Device::Ddc { bus, max },
                        raw: Some(raw),
                    })
                })
            }
        });
        hardware.unwrap_or(Found {
            device: if target.has_gamma {
                Device::Software
            } else {
                Device::None
            },
            raw: None,
        })
    }

    /// `cardN` for a card or render node, through `/sys/dev/char`.
    pub fn card_name(&self, node: CardNode) -> Option<String> {
        let drm = self
            .root
            .join(format!("dev/char/{}:{}/device/drm", node.major, node.minor));
        entry_names(&drm).into_iter().find(|name| is_card(name))
    }

    pub fn connector_directory(&self, card: &str, connector: &str) -> PathBuf {
        self.root.join(format!("class/drm/{card}-{connector}"))
    }

    fn backlight_under(&self, connector: &Path) -> Option<String> {
        entry_names(connector)
            .into_iter()
            .find(|name| connector.join(name).join("max_brightness").is_file())
    }

    fn machine_backlight(&self) -> Option<String> {
        let names = entry_names(&self.backlight_class());
        BACKLIGHT_TYPES.iter().find_map(|wanted| {
            names
                .iter()
                .find(|name| self.backlight_type(name).as_deref() == Some(*wanted))
                .cloned()
        })
    }

    fn backlight_type(&self, name: &str) -> Option<String> {
        let kind = fs::read_to_string(self.backlight_class().join(name).join("type")).ok()?;
        Some(kind.trim().to_owned())
    }

    /// A number from `/sys/class/backlight/<name>/<file>`.
    pub fn backlight_value(&self, name: &str, file: &str) -> Option<u32> {
        fs::read_to_string(self.backlight_class().join(name).join(file))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    fn backlight_class(&self) -> PathBuf {
        self.root.join("class/backlight")
    }

    /// The connector's DDC bus first, then each `i2c-N` under it (DP AUX).
    fn ddc_buses(&self, connector: &Path) -> Vec<u32> {
        let linked = fs::read_link(connector.join("ddc"))
            .ok()
            .and_then(|target| bus_number(target.file_name()?.to_str()?));
        let mut buses: Vec<u32> = linked.into_iter().collect();
        for bus in entry_names(connector)
            .iter()
            .filter_map(|name| bus_number(name))
        {
            if !buses.contains(&bus) {
                buses.push(bus);
            }
        }
        buses
    }
}

fn is_card(name: &str) -> bool {
    name.strip_prefix("card")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

fn bus_number(name: &str) -> Option<u32> {
    name.strip_prefix("i2c-")?.parse().ok()
}

/// The entries of a directory, sorted, so a probe gives the same answer each
/// time.
fn entry_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests;

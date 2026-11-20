//! The brightness of each output, owned by the compositor.
//!
//! A built-in panel has a backlight, an external monitor may answer over
//! DDC/CI, and any other output with a gamma ramp is dimmed in that ramp.
//! Whichever it is, clients see one perceptual level from 0 to [`FULL`]. The
//! model here changes at once; slow hardware catches up in [`worker`].

mod backlight;
mod curve;
mod ddc;
mod device;
mod error;
mod hardware;
mod level;
mod worker;

use std::collections::HashMap;

use tokio::sync::mpsc::{self, UnboundedSender};

pub use device::{CardNode, Device, Found, ProbeTarget};
pub use level::{FULL, Reading, software_level};
pub use worker::{Operation, Outcome};

use crate::{
    controls::Step,
    utils::{OutputId, runtime::TaskSender},
};
use hardware::SystemHardware;
use worker::Job;

const DEBUG_VARIABLE: &str = "CROWN_DEBUG_BRIGHTNESS";

struct OutputBrightness {
    device: Device,
    level: u32,
    /// Counts the levels set, so a read that started before the newest one
    /// cannot undo it.
    sequence: u64,
}

impl OutputBrightness {
    fn reading(&self) -> Reading {
        Reading {
            control: self.device.control(),
            level: self.level,
        }
    }
}

pub struct Brightness {
    outputs: HashMap<OutputId, OutputBrightness>,
    jobs: UnboundedSender<Job>,
    /// What a nested output reports: nothing, unless a test asks for a
    /// software dim it can see in the protocol but not on screen.
    nested: Device,
    software_changed: bool,
}

impl Brightness {
    /// Starts the hardware worker; its outcomes come back to
    /// [`State::receive_brightness`](crate::state::State::receive_brightness).
    pub fn start(tasks: &TaskSender) -> Self {
        let (jobs, requests) = mpsc::unbounded_channel();
        let deliver = tasks.clone();
        tasks.spawn(worker::work(
            requests,
            SystemHardware::default(),
            move |outcome| deliver.deliver(move |state| state.receive_brightness(outcome)),
        ));
        Self::with_jobs(jobs, nested_device_from_environment())
    }

    fn with_jobs(jobs: UnboundedSender<Job>, nested: Device) -> Self {
        Self {
            outputs: HashMap::new(),
            jobs,
            nested,
            software_changed: false,
        }
    }

    /// The device a nested output reports.
    pub fn nested_device(&self) -> Device {
        self.nested.clone()
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the protocol and the keys land")
    )]
    pub fn reading(&self, output: OutputId) -> Reading {
        self.outputs
            .get(&output)
            .map_or(Reading::UNCONTROLLED, OutputBrightness::reading)
    }

    /// Starts tracking an output whose device is already known.
    pub fn adopt(&mut self, output: OutputId, device: Device, level: u32) {
        self.software_changed |= device == Device::Software;
        self.outputs.insert(
            output,
            OutputBrightness {
                device,
                level,
                sequence: 0,
            },
        );
    }

    /// Starts tracking a KMS output. It has no control until the probe
    /// answers, so a software dim saved for a monitor that turns out to answer
    /// over DDC/CI never flashes on.
    pub fn probe(&mut self, target: ProbeTarget) {
        self.adopt(target.output, Device::None, FULL);
        self.send(Job::Probe(target));
    }

    /// Takes the probe's answer. `software_level` is the level a software
    /// dim starts at. Returns the new reading of an output still tracked.
    pub fn probed(
        &mut self,
        output: OutputId,
        found: Found,
        software_level: u32,
    ) -> Option<Reading> {
        let entry = self.outputs.get_mut(&output)?;
        entry.level = match (found.device.scale(), found.raw) {
            (Some((curve, max)), Some(raw)) => curve.level(raw, max),
            _ if found.device == Device::Software => software_level,
            _ => FULL,
        };
        entry.device = found.device;
        self.software_changed |= entry.device == Device::Software;
        Some(entry.reading())
    }

    pub fn forget(&mut self, output: OutputId) {
        self.outputs.remove(&output);
    }

    /// Sets a level, raised to the control's floor. `None` when the level
    /// of `output` cannot change.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the protocol and the keys land")
    )]
    pub fn set(&mut self, output: OutputId, level: u32) -> Option<Reading> {
        let entry = self.outputs.get_mut(&output)?;
        entry.level = entry.device.control().clamp(level)?;
        entry.sequence += 1;
        let reading = entry.reading();
        match entry.device.scale() {
            Some((curve, max)) => {
                let job = Job::Write {
                    output,
                    device: entry.device.clone(),
                    raw: curve.raw(entry.level, max),
                };
                self.send(job);
            }
            None => self.software_changed = true,
        }
        Some(reading)
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the protocol and the keys land")
    )]
    pub fn step(&mut self, output: OutputId, step: Step) -> Option<Reading> {
        let current = self.outputs.get(&output)?.level;
        self.set(output, level::stepped(current, step))
    }

    /// Reads the hardware again, for a change made with a monitor's own
    /// buttons or by another program.
    pub fn refresh(&mut self, output: OutputId) {
        let Some(entry) = self.outputs.get(&output) else {
            return;
        };
        if entry.device.scale().is_some() {
            let job = Job::Read {
                output,
                sequence: entry.sequence,
                device: entry.device.clone(),
            };
            self.send(job);
        }
    }

    /// Takes a value read from the hardware. Returns the new reading when the
    /// level changed: a value the current level already maps to keeps it,
    /// and a value read before a newer level was set is dropped.
    pub fn read_back(&mut self, output: OutputId, sequence: u64, raw: u32) -> Option<Reading> {
        let entry = self.outputs.get_mut(&output)?;
        let (curve, max) = entry.device.scale()?;
        if entry.sequence != sequence || curve.raw(entry.level, max) == raw {
            return None;
        }
        entry.level = curve.level(raw, max);
        Some(entry.reading())
    }

    /// The share of full brightness the gamma ramp of `output` keeps.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the protocol and the keys land")
    )]
    pub fn software_dim(&self, output: OutputId) -> f64 {
        match self.outputs.get(&output) {
            Some(entry) if entry.device == Device::Software => {
                f64::from(entry.level) / f64::from(FULL)
            }
            _ => 1.0,
        }
    }

    /// Whether a software level changed since the last call, so the ramps are
    /// written once for a burst of changes.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used once the protocol and the keys land")
    )]
    pub fn take_software_changed(&mut self) -> bool {
        std::mem::take(&mut self.software_changed)
    }

    fn send(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::warn!("the brightness worker is gone");
        }
    }
}

/// `CROWN_DEBUG_BRIGHTNESS=software` has nested outputs report a software
/// dim, so the protocol can be tested without hardware.
fn nested_device_from_environment() -> Device {
    match std::env::var(DEBUG_VARIABLE).as_deref() {
        Ok("software") => Device::Software,
        _ => Device::None,
    }
}

#[cfg(test)]
mod tests;

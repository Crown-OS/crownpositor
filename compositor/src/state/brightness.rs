//! Where the brightness model meets the outputs: which device an output
//! gets when it appears, and what the worker's answers change.

use smithay::output::Output;

use crate::utils::{head_name::unqualified, output_setting::output_setting};
use crate::{
    brightness::{Control, Device, FULL, Operation, Outcome, ProbeTarget, Reading, software_level},
    controls::{Gauge, Level, Step},
    shell::output_kind::OutputKind,
    state::State,
    utils::OutputId,
};

impl State {
    /// Gives a new output its device: probed on a KMS card, decided at once
    /// for nested and virtual outputs.
    pub(super) fn track_output_brightness(&mut self, output: &Output) {
        let Some(monitor) = self.shell.monitor(output) else {
            return;
        };
        let id = monitor.id();
        let config = monitor.config();
        let card = self.backend.card_node(output);
        match (config.kind, card) {
            (OutputKind::Physical, Some(card)) => {
                let target = ProbeTarget {
                    output: id,
                    card,
                    connector: unqualified(&config.name).to_owned(),
                    has_gamma: self.backend.gamma_size(output).is_some(),
                };
                self.brightness.probe(target);
            }
            (OutputKind::Nested, _) => {
                let level = self.saved_software_level(id);
                let device = self.brightness.nested_device();
                self.brightness.adopt(id, device, level);
            }
            _ => self.brightness.adopt(id, Device::None, FULL),
        }
    }

    /// Takes what the brightness worker found or failed at.
    pub fn receive_brightness(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Probed { output, found } => {
                let level = self.saved_software_level(output);
                if let Some(reading) = self.brightness.probed(output, found, level) {
                    tracing::info!(%output, ?reading, "output brightness found");
                    self.brightness_changed(output, reading);
                }
            }
            Outcome::Level {
                output,
                sequence,
                raw,
            } => {
                if let Some(reading) = self.brightness.read_back(output, sequence, raw) {
                    self.brightness_changed(output, reading);
                }
            }
            Outcome::Failed {
                output,
                during,
                error,
            } => {
                tracing::warn!(%output, ?during, %error, "output brightness");
                if during == Operation::Write {
                    self.brightness.refresh(output);
                }
            }
        }
    }

    /// A brightness key: steps the monitor under the pointer and shows its
    /// new level.
    pub fn step_focused_brightness(&mut self, step: Step) {
        let Some(output) = self.shell.focused_monitor().map(|monitor| monitor.id()) else {
            return;
        };
        let Some(reading) = self.brightness.step(output, step) else {
            return;
        };
        self.brightness_changed(output, reading);
        self.remember_software_brightness(reading);
        self.common.controls.show(Level {
            gauge: Gauge::Display,
            percent: (reading.level * 100 / FULL) as u8,
            muted: false,
        });
    }

    /// Saves a software level the user set; hardware keeps its own.
    pub fn remember_software_brightness(&mut self, reading: Reading) {
        if reading.control == Control::Software {
            self.schedule_persist();
        }
    }

    fn brightness_changed(&mut self, output: OutputId, reading: Reading) {
        if let Some(output) = self
            .shell
            .monitor_by_id(output)
            .map(|monitor| monitor.output().clone())
        {
            self.announce_brightness(&output, reading);
        }
    }

    /// The software level `compositor.ron` keeps for an output.
    fn saved_software_level(&self, output: OutputId) -> u32 {
        let saved = self.shell.monitor_by_id(output).and_then(|monitor| {
            let config = monitor.config();
            output_setting(
                &self.config.current,
                &config.name,
                config.connector.as_str(),
            )?
            .software_brightness
        });
        software_level(saved)
    }
}

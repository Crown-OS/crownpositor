//! Programming each output's colour state onto the hardware.
//!
//! Night light and the dim are the settings the compositor, rather than a
//! monitor's own OSD, has to carry out. On SDR both are a gamma ramp (see
//! [`crate::color::night_light`] for the arithmetic), held per output in
//! [`OutputColor`], so what is left here is which outputs get one, and what
//! happens to a `zwlr_gamma_control_v1` client that wanted the same output.
//!
//! ## Who owns gamma
//!
//! Only one party can. The rule is that the compositor takes an output's gamma
//! exactly while the user has asked for something (dimmed, or warmed) and
//! leaves it alone otherwise. So `gammastep` works on a stock install, loses
//! the output the moment night light is switched on in Settings, and gets it
//! back when that is switched off again. Anything subtler would mean two
//! things writing one lookup table and the screen's colour depending on which
//! wrote last.

use smithay::{
    output::Output,
    reexports::wayland_server::{Resource, protocol::wl_output::WlOutput},
};

use protocols::gamma_control::GammaRamps;

use crate::{
    color::{night_light, output::OutputColor},
    state::State,
};

/// The ramps the compositor last wrote to each output's CRTC.
///
/// An output with an entry is owned: a gamma client cannot have it. "No
/// ramp" and "identity ramp" are different acts: the first leaves a gamma
/// client's table alone, the second overwrites it. Remembering what was
/// written tells the two apart and skips writing a ramp the CRTC already
/// holds.
#[derive(Debug, Default)]
pub struct DisplayGamma {
    programmed: Vec<(String, GammaRamps)>,
}

impl DisplayGamma {
    pub fn owns(&self, output: &Output) -> bool {
        self.programmed(output).is_some()
    }

    fn programmed(&self, output: &Output) -> Option<&GammaRamps> {
        let name = output.name();
        self.programmed
            .iter()
            .find(|(held, _)| *held == name)
            .map(|(_, ramps)| ramps)
    }

    fn record(&mut self, output: &Output, ramps: Option<GammaRamps>) {
        let name = output.name();
        self.programmed.retain(|(held, _)| *held != name);
        if let Some(ramps) = ramps {
            self.programmed.push((name, ramps));
        }
    }

    /// The CRTC lost its table (a modeset, a VT switch, a power cycle), so
    /// the next [`State::apply_output_color`] writes it again.
    pub fn forget(&mut self, output: &Output) {
        self.record(output, None);
    }

    pub fn forget_all(&mut self) {
        self.programmed.clear();
    }
}

impl State {
    /// Brings every output's colour state in line with the settings, and its
    /// gamma with that.
    ///
    /// Idempotent, and cheap enough to call on anything that could change the
    /// answer: a settings edit, a hotplug, or an output configuration. A ramp
    /// is written only when it differs from what the CRTC holds.
    pub fn apply_output_color(&mut self) {
        let warmth = night_light::warmth(&self.config.current.display);
        let outputs: Vec<Output> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| monitor.output().clone())
            .collect();

        let mut programmed = false;
        for output in outputs {
            let dim = self.output_dim(&output);
            let Some(monitor) = self.shell.monitor_mut(&output) else {
                continue;
            };
            let before = *monitor.color();
            let color = monitor.color_mut();
            color.warmth = warmth;
            color.dim = dim;
            let color = *color;

            if color.describes_differently(&before) {
                self.announce_output_description(&output);
            }
            programmed |= self.program_gamma(&output, &color);
        }

        // The ramps are consulted during scanout, so nothing is re-rendered,
        // but an idle desktop has no page flip coming to consult them with.
        if programmed {
            self.queue_redraw();
        }
    }

    /// The share of full brightness `output` keeps.
    pub fn output_dim(&self, _output: &Output) -> f64 {
        night_light::dim(&self.config.current.display)
    }

    /// Writes `output`'s ramp when it differs from what its CRTC holds,
    /// reporting whether anything was written.
    fn program_gamma(&mut self, output: &Output, color: &OutputColor) -> bool {
        let Some(size) = self.backend.gamma_size(output) else {
            return false;
        };
        let wanted = color.ramps(size);
        if self.display_gamma.programmed(output) == wanted.as_ref() {
            return false;
        }

        if wanted.is_some() {
            self.wayland
                .gamma_control_state
                .revoke_matching(|held| Output::from_resource(held).as_ref() == Some(output));
        }
        let written = self.backend.set_gamma(output, wanted.as_ref());
        if written {
            self.display_gamma.record(output, wanted);
        }
        written
    }

    /// Tells the clients watching `output`'s colours that its description
    /// changed, and has every surface's preferred description looked at
    /// again.
    fn announce_output_description(&mut self, output: &Output) {
        let protocol = &mut self.wayland.color_management_state;
        let mut watched: Vec<WlOutput> = Vec::new();
        for wl_output in protocol.watched_outputs() {
            if output.owns(wl_output) && !watched.contains(wl_output) {
                watched.push(wl_output.clone());
            }
        }
        for wl_output in &watched {
            protocol.output_description_changed(wl_output);
            if wl_output.version() >= 2 {
                wl_output.done();
            }
        }
        self.shell.mark_preferred_stale();
    }
}

#[cfg(test)]
mod tests {
    use smithay::output::{PhysicalProperties, Subpixel};

    use super::*;

    fn output(name: &str) -> Output {
        Output::new(
            name.into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: String::new(),
                model: String::new(),
                serial_number: String::new(),
            },
        )
    }

    fn ramps(level: u16) -> GammaRamps {
        GammaRamps {
            red: vec![0, level],
            green: vec![0, level],
            blue: vec![0, level],
        }
    }

    #[test]
    fn an_output_is_owned_while_a_ramp_is_recorded_for_it() {
        let (left, right) = (output("DP-1"), output("DP-2"));
        let mut gamma = DisplayGamma::default();
        gamma.record(&left, Some(ramps(100)));
        assert!(gamma.owns(&left));
        assert!(!gamma.owns(&right));

        gamma.record(&left, Some(ramps(200)));
        assert_eq!(gamma.programmed(&left), Some(&ramps(200)));

        gamma.record(&left, None);
        assert!(!gamma.owns(&left));
    }

    #[test]
    fn a_forgotten_ramp_is_written_again() {
        let (left, right) = (output("DP-1"), output("DP-2"));
        let mut gamma = DisplayGamma::default();
        gamma.record(&left, Some(ramps(100)));
        gamma.record(&right, Some(ramps(100)));

        gamma.forget(&left);
        assert_eq!(gamma.programmed(&left), None);
        assert!(gamma.owns(&right));

        gamma.forget_all();
        assert!(!gamma.owns(&right));
    }
}

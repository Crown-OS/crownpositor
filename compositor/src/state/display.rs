//! Programming the Display page's settings onto the hardware.
//!
//! Brightness and night light are the two settings in `display.ron` that the
//! compositor, rather than a monitor's own OSD, has to carry out. Both are a
//! gamma ramp — see [`crate::color::night_light`] for the arithmetic — so all
//! that is left here is which outputs get one, and what happens to a
//! `zwlr_gamma_control_v1` client that wanted the same output.
//!
//! ## Who owns gamma
//!
//! Only one party can. The rule is that the compositor takes an output's gamma
//! exactly while the user has asked for something — dimmed, or warmed — and
//! leaves it alone otherwise. So `gammastep` works on a stock install, loses
//! the output the moment night light is switched on in Settings, and gets it
//! back when that is switched off again. Anything subtler would mean two
//! things writing one lookup table and the screen's colour depending on which
//! wrote last.

use smithay::output::Output;

use crate::{color::night_light, state::State};

/// The outputs the compositor's own ramps are currently programmed on.
///
/// Needed because "no ramp" and "identity ramp" are different acts: the first
/// leaves a gamma client's table alone, the second overwrites it. Without
/// remembering which outputs were taken, turning night light off could only
/// be implemented as one of the two, and both are wrong for half the cases.
#[derive(Debug, Default)]
pub struct DisplayGamma {
    owned: Vec<String>,
}

impl DisplayGamma {
    /// Records the new ownership and says whether it changed.
    fn set(&mut self, output: &Output, owned: bool) -> bool {
        let name = output.name();
        let held = self.owned.iter().position(|held| *held == name);

        match (held, owned) {
            (None, true) => self.owned.push(name),
            (Some(index), false) => drop(self.owned.swap_remove(index)),
            _ => return false,
        }
        true
    }
}

impl State {
    /// Brings every output's gamma in line with `display.ron`.
    ///
    /// Idempotent, and cheap enough to call on anything that could change the
    /// answer: a settings edit, a hotplug, or an output configuration.
    pub fn apply_display_gamma(&mut self) {
        let outputs: Vec<Output> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| monitor.output().clone())
            .collect();

        let mut programmed = false;
        for output in outputs {
            programmed |= self.apply_output_gamma(&output);
        }

        // The ramps are consulted during scanout, so nothing is re-rendered —
        // but an idle desktop has no page flip coming to consult them with.
        if programmed {
            self.queue_redraw();
        }
    }

    /// One output's share of that, reporting whether anything was written.
    fn apply_output_gamma(&mut self, output: &Output) -> bool {
        let Some(size) = self.backend.gamma_size(output) else {
            return false;
        };

        let ramps = night_light::ramps(size, &self.config.current.display);
        let wanted = ramps.is_some();
        let changed = self.display_gamma.set(output, wanted);

        // Nothing is being asked of this output and nothing was taken from
        // it, so whoever holds its gamma keeps it.
        if !wanted && !changed {
            return false;
        }

        if wanted {
            self.wayland
                .gamma_control_state
                .revoke_matching(|held| Output::from_resource(held).as_ref() == Some(output));
        }
        self.backend.set_gamma(output, ramps.as_ref())
    }
}

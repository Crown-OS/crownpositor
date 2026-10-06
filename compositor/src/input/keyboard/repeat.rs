//! Repeat for held hardware controls.
//!
//! Clients repeat their own keys, but a key the compositor intercepted never
//! reaches one, so a held volume key would step once. This steps it again on a
//! timer for as long as the key stays down.

use std::time::Duration;

use calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use smithay::input::keyboard::Keycode;

use crate::{controls::Control, input::shortcuts::Action, state::State};

/// Long enough that a tap is one step.
const DELAY: Duration = Duration::from_millis(300);
/// About twelve steps a second: 5% steps sweep the range in under two seconds
/// without overshooting the moment the key comes up.
const INTERVAL: Duration = Duration::from_millis(80);

pub struct HeldControl {
    key: Keycode,
    timer: RegistrationToken,
}

impl State {
    pub(super) fn hold_control(&mut self, key: Keycode, control: Control) {
        self.release_held_control();
        if !control.repeats() {
            return;
        }
        let timer = self.common.event_loop_handle.insert_source(
            Timer::from_duration(DELAY),
            move |_, _, state| {
                state.handle_action(Action::Control(control));
                TimeoutAction::ToDuration(INTERVAL)
            },
        );
        match timer {
            Ok(timer) => self.input.held_control = Some(HeldControl { key, timer }),
            Err(error) => tracing::warn!(%error, "cannot repeat a held control"),
        }
    }

    pub(super) fn release_control_key(&mut self, key: Keycode) {
        if self
            .input
            .held_control
            .as_ref()
            .is_some_and(|held| held.key == key)
        {
            self.release_held_control();
        }
    }

    fn release_held_control(&mut self) {
        if let Some(held) = self.input.held_control.take() {
            self.common.event_loop_handle.remove(held.timer);
        }
    }
}

use smithay::{
    delegate_idle_notify,
    wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState},
};

use crate::state::State;

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.wayland.idle_notifier_state
    }
}

delegate_idle_notify!(State);

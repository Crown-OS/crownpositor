use smithay::{
    delegate_kde_decoration,
    wayland::shell::kde::decoration::{KdeDecorationHandler, KdeDecorationState},
};

use crate::state::State;

impl KdeDecorationHandler for State {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.wayland.kde_decoration_state
    }
}

delegate_kde_decoration!(State);

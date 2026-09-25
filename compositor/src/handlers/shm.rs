use smithay::wayland::shm::{ShmHandler, ShmState};

use crate::state::State;

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.wayland.shm_state
    }
}

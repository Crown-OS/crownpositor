use smithay::{
    delegate_session_lock,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    wayland::session_lock::{
        LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
    },
};

use crate::state::State;

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.wayland.session_lock_manager_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        // TODO: only confirm once every output is covered by a lock surface.
        confirmation.lock();
    }

    fn unlock(&mut self) {}

    fn new_surface(&mut self, _surface: LockSurface, _output: WlOutput) {}
}

delegate_session_lock!(State);

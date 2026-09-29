use smithay::{
    output::Output,
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

    /// Hides the desktop at once; the client hears `locked` only after every
    /// output has drawn that, in `confirm_session_lock`.
    fn lock(&mut self, confirmation: SessionLocker) {
        self.dismiss_menu();
        for monitor in self.shell.monitors_mut() {
            monitor.spacecontrol_mut().close();
        }
        self.shell.session_lock.begin(confirmation);
        self.queue_redraw();
    }

    fn unlock(&mut self) {
        self.shell.session_lock.end();
        tracing::info!("session unlocked");
        self.update_keyboard_focus();
        self.queue_redraw();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            return;
        };
        if let Some(monitor) = self.shell.monitor(&output) {
            let size = monitor.geometry().size;
            surface.with_pending_state(|state| {
                state.size = Some((size.w as u32, size.h as u32).into());
            });
            surface.send_configure();
        }
        self.shell.session_lock.add_surface(output, surface);
        self.update_keyboard_focus();
        self.queue_redraw();
    }
}

impl State {
    /// Confirms a pending lock once every output a backend draws has shown a
    /// locked frame. Virtual outputs only draw on demand, so they cannot hold
    /// the confirmation hostage.
    pub fn confirm_session_lock(&mut self) {
        let virtual_outputs = &self.virtual_outputs;
        let drawn = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| monitor.output())
            .filter(|output| !virtual_outputs.contains(output));
        let outputs: Vec<_> = drawn.cloned().collect();
        self.shell.session_lock.confirm_if_blanked(outputs.iter());
    }
}

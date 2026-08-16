use smithay::{
    delegate_idle_inhibit, reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::idle_inhibit::IdleInhibitHandler,
};

use crate::state::State;

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.wayland.idle_inhibiting_surfaces.insert(surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.wayland.idle_inhibiting_surfaces.remove(&surface);
    }
}

delegate_idle_inhibit!(State);

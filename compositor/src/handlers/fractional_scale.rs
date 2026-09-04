use smithay::{
    delegate_fractional_scale, reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::fractional_scale::FractionalScaleHandler,
};

use crate::state::State;

impl FractionalScaleHandler for State {
    /// A client may create the object before it ever commits, so the scale it
    /// would otherwise learn on commit has to be sent here too.
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        self.shell.advertise_scale(&surface);
    }
}

delegate_fractional_scale!(State);

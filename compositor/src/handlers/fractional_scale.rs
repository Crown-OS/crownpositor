use smithay::{
    delegate_fractional_scale, reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::fractional_scale::FractionalScaleHandler,
};

use crate::state::State;

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, _surface: WlSurface) {
        // TODO: send the scale of the output the surface is currently on.
    }
}

delegate_fractional_scale!(State);

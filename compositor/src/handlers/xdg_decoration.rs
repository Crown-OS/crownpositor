use smithay::{
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    wayland::shell::xdg::{ToplevelSurface, decoration::XdgDecorationHandler},
};

use crate::{shell::tile::Chrome, state::State};

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.set_decoration_mode(&toplevel, Mode::ServerSide);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        self.set_decoration_mode(&toplevel, mode);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.set_decoration_mode(&toplevel, Mode::ServerSide);
    }
}

impl State {
    /// Grants the mode and hands the frame to whoever now draws it. A mapped
    /// window that changes its mind loses or gains our titlebar at once, and
    /// the relayout resizes its client area to match.
    fn set_decoration_mode(&mut self, toplevel: &ToplevelSurface, mode: Mode) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(mode));
        toplevel.send_configure();
        if self
            .shell
            .set_window_chrome(toplevel.wl_surface(), Chrome::for_mode(Some(mode)))
        {
            self.shell.refresh();
            self.queue_redraw();
        }
    }
}

use smithay::{
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    wayland::shell::xdg::{ToplevelSurface, decoration::XdgDecorationHandler},
};

use super::kde_decoration;
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

/// Who the client asked to draw its frame, through whichever protocol it used.
pub fn requested_chrome(toplevel: &ToplevelSurface) -> Chrome {
    Chrome::requested(
        toplevel.with_pending_state(|state| state.decoration_mode),
        kde_decoration::requested_mode(toplevel.wl_surface()),
    )
}

impl State {
    fn set_decoration_mode(&mut self, toplevel: &ToplevelSurface, mode: Mode) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(mode));
        toplevel.send_configure();
        self.follow_decoration_request(toplevel);
    }

    /// Hands the frame to whoever the client now asks to draw it. A mapped
    /// window that changes its mind loses or gains our titlebar at once, and
    /// the relayout resizes its client area to match.
    pub(super) fn follow_decoration_request(&mut self, toplevel: &ToplevelSurface) {
        if self
            .shell
            .set_window_chrome(toplevel.wl_surface(), requested_chrome(toplevel))
        {
            self.shell.refresh();
            self.queue_redraw();
        }
    }
}

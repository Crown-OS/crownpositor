use smithay::{
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    wayland::{
        compositor::with_states,
        shell::xdg::{SurfaceCachedState, ToplevelSurface, decoration::XdgDecorationHandler},
    },
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
        let chrome = requested_chrome(toplevel);
        if self.shell.set_window_chrome(toplevel.wl_surface(), chrome) {
            if chrome == Chrome::Compositor {
                forget_window_geometry(toplevel.wl_surface());
            }
            self.shell.refresh();
            self.queue_redraw();
        }
    }
}

/// Drops the window geometry a client set around its own shadows. Chrome
/// keeps that geometry when it gives the frame to us and then fills its whole
/// buffer, so the stale inset moves the content up-left and crops it. The
/// geometry is cached in both halves because a commit copies pending into
/// current, and a geometry the client sends after this still applies.
fn forget_window_geometry(surface: &WlSurface) {
    with_states(surface, |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        cached.pending().geometry = None;
        cached.current().geometry = None;
    });
}

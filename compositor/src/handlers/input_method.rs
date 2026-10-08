//! `zwp_input_method_v2`, the IME side of `zwp_text_input_v3`: fcitx5 or IBus
//! composes text for the focused client. Keyboard focus already drives
//! text-input enter/leave inside smithay, so all that is left here is the
//! candidate popup, which rides the window it was opened over.

use smithay::{
    desktop::{PopupKind, PopupManager, Window},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
    wayland::input_method::{InputMethodHandler, PopupSurface},
};

use crate::state::State;

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(err) = self.shell.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!(%err, "failed to track an input method popup");
        }
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn popup_repositioned(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            self.queue_redraw_for_surface(&parent);
        }
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.shell
            .window_for_surface(parent)
            .map(Window::geometry)
            .unwrap_or_default()
    }
}

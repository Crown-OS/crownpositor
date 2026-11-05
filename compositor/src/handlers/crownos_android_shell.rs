//! Delegation glue for `crownos_android_shell_v1`. The bookkeeping lives in
//! [`protocols::crownos_android_shell`]; here are Back, the extras a tile
//! takes from an Android task window, and a splash handing its place over.

use std::time::Instant;

use crownos_protocols::android_shell::v1::server::{
    crownos_android_shell_manager_v1::CrownosAndroidShellManagerV1,
    crownos_android_toplevel_v1::CrownosAndroidToplevelV1,
};
use protocols::crownos_android_shell::{
    AndroidDetails, AndroidPeer, AndroidShellGlobalData, AndroidShellHandler, AndroidShellState,
};
use smithay::{
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel,
    wayland::shell::xdg::ToplevelSurface,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{
    handlers::seat::KeyboardFocusTarget,
    shell::{android::AndroidWindow, tile::Tile},
    state::State,
    utils::id::WindowId,
};

impl AndroidShellHandler for State {
    fn android_shell_state(&mut self) -> &mut AndroidShellState {
        &mut self.wayland.android_shell_state
    }

    /// A window that has not mapped yet picks its details up as it maps.
    fn android_details_changed(
        &mut self,
        toplevel: &XdgToplevel,
        details: Option<&AndroidDetails>,
    ) {
        let Some(id) = self
            .shell
            .xdg_shell_state
            .get_toplevel(toplevel)
            .and_then(|toplevel| self.shell.window_id(toplevel.wl_surface()))
        else {
            return;
        };
        if self
            .shell
            .set_android_window(id, details.map(AndroidWindow::from))
        {
            self.queue_redraw();
        }
    }
}

impl State {
    /// Tells window `id` the user pressed Back. `false` when it is no Android
    /// task window, which leaves whatever pressed Back to do something else.
    pub fn send_android_back(&self, id: WindowId) -> bool {
        self.shell
            .tile(id)
            .and_then(Tile::toplevel)
            .is_some_and(|toplevel| {
                self.wayland
                    .android_shell_state
                    .back(toplevel.xdg_toplevel())
            })
    }

    /// The Android task window holding the keyboard, if one does: where the
    /// Back shortcut goes.
    pub fn android_window_with_keyboard(&self) -> Option<WindowId> {
        let KeyboardFocusTarget::Window(window) = self.shell.keyboard_focus()? else {
            return None;
        };
        let id = self.window_id_of(&window)?;
        self.shell
            .tile(id)
            .is_some_and(Tile::is_android)
            .then_some(id)
    }

    /// The extras `toplevel` committed before it mapped, if it is an Android
    /// task window.
    pub(super) fn android_window_of(&self, toplevel: &ToplevelSurface) -> Option<AndroidWindow> {
        self.wayland
            .android_shell_state
            .details(toplevel.xdg_toplevel())
            .map(AndroidWindow::from)
    }

    /// The placeholder an Android task window takes the place of as it maps:
    /// the daemon's splash for the same app_id, marked in the last two
    /// minutes. Consumes the mark.
    pub(super) fn placeholder_for(
        &mut self,
        toplevel: &ToplevelSurface,
        app_id: Option<&str>,
    ) -> Option<WindowId> {
        let app_id = app_id?;
        let android_shell = &mut self.wayland.android_shell_state;
        android_shell.details(toplevel.xdg_toplevel())?;
        let placeholder = android_shell.take_placeholder(app_id, Instant::now())?;
        let surface = self.shell.xdg_shell_state.get_toplevel(&placeholder)?;
        self.shell.window_id(surface.wl_surface())
    }

    /// Puts `tile` in `placeholder`'s place, in its state, and closes the
    /// placeholder. It leaves the model at once, so it neither shows nor
    /// plays a close of its own: the window that replaced it is already
    /// there. Hands `tile` back when the placeholder is gone.
    pub(super) fn replace_placeholder(
        &mut self,
        placeholder: WindowId,
        mut tile: Tile,
    ) -> Option<Tile> {
        let Some(area) = self
            .shell
            .location(placeholder)
            .and_then(|location| self.shell.workspace(location))
            .map(|workspace| workspace.area())
        else {
            return Some(tile);
        };
        let Some(old) = self.shell.tile(placeholder) else {
            return Some(tile);
        };
        tile.take_place_of(old, area);
        match self.shell.replace_tile(placeholder, tile) {
            Ok(replaced) => {
                self.shell.forget_carried(placeholder);
                self.shell.menus.forget(placeholder);
                if let Some(toplevel) = replaced.toplevel() {
                    toplevel.send_close();
                }
                None
            }
            Err(tile) => Some(*tile),
        }
    }
}

delegate_global_dispatch!(State: [CrownosAndroidShellManagerV1: AndroidShellGlobalData] => AndroidShellState);
delegate_dispatch!(State: [CrownosAndroidShellManagerV1: Option<AndroidPeer>] => AndroidShellState);
delegate_dispatch!(State: [CrownosAndroidToplevelV1: ()] => AndroidShellState);

use smithay::{
    delegate_xdg_shell,
    desktop::{PopupKind, Window},
    reexports::wayland_server::protocol::{wl_seat::WlSeat, wl_surface::WlSurface},
    utils::Serial,
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::state::{ShellState, State};

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.shell.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);

        let position = self.shell.layout_manager.add_window(window.clone());
        self.shell.space.map_element(window, position, false);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let window = self.shell.window_for_surface(surface.wl_surface()).cloned();
        if let Some(window) = window {
            self.shell.space.unmap_elem(&window);
            // TODO:
            // self.shell.layout_manager.remove_window(window);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // TODO: unconstrain the popup against its output before tracking it.
        let _ = self.shell.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {}
}

/// Sends the initial configure once a surface has committed for the first time.
pub fn handle_commit(shell: &mut ShellState, surface: &WlSurface) {
    if let Some(toplevel) = shell.window_for_surface(surface).and_then(Window::toplevel) {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        if !initial_configure_sent {
            toplevel.send_configure();
        }
    }

    shell.popups.commit(surface);

    if let Some(PopupKind::Xdg(popup)) = shell.popups.find_popup(surface)
        && !popup.is_initial_configure_sent()
    {
        popup.send_configure().expect("initial configure failed");
    }
}

delegate_xdg_shell!(State);

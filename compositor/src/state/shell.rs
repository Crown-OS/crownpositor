use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    utils::{Logical, Point},
    wayland::shell::{wlr_layer::WlrLayerShellState, xdg::XdgShellState},
};

use crate::state::State;

pub struct ShellState {
    pub xdg_shell_state: XdgShellState,
    pub space: Space<Window>,
    pub popups: PopupManager,
    pub layer_shell: WlrLayerShellState,
}

impl ShellState {
    pub fn try_new(display: &DisplayHandle) -> anyhow::Result<Self> {
        Ok(Self {
            layer_shell: WlrLayerShellState::new::<State>(&display),
            xdg_shell_state: XdgShellState::new::<State>(&display),
            space: Space::default(),
            popups: PopupManager::default(),
        })
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<&Window> {
        self.space
            .elements()
            .find(|window| window.toplevel().is_some_and(|t| t.wl_surface() == surface))
    }

    pub fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(location)
            .and_then(|(window, window_location)| {
                window
                    .surface_under(location - window_location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(surface, offset)| (surface, (offset + window_location).to_f64()))
            })
    }
}

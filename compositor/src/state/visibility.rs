use crate::state::State;

impl State {
    /// Tells `crownos_surface_visibility_v1` observers whatever changed since
    /// the last scene update.
    pub fn refresh_surface_visibility(&mut self) {
        let shell = &self.shell;
        self.wayland
            .surface_visibility_state
            .refresh(|surface| shell.surface_visibility(surface));
    }
}

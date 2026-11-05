use crate::state::State;

impl State {
    /// Tells `crownos_surface_visibility_v1` observers whatever changed since
    /// the last scene update, and suspends or resumes the windows that went
    /// out of sight or came back.
    pub fn refresh_surface_visibility(&mut self) {
        let shell = &self.shell;
        self.wayland
            .surface_visibility_state
            .refresh(|surface| shell.surface_visibility(surface));
        self.shell.refresh_suspended();
    }
}

use smithay::utils::IsAlive;

use crate::state::State;

impl State {
    /// An inhibitor only holds while its surface is on screen: a video player
    /// left on another workspace must not keep the display awake.
    pub fn refresh_idle_inhibit(&mut self) {
        let surfaces = &mut self.wayland.idle_inhibiting_surfaces;
        surfaces.retain(IsAlive::alive);
        let inhibited = surfaces
            .iter()
            .any(|surface| self.shell.is_surface_visible(surface));
        self.wayland.idle_notifier_state.set_is_inhibited(inhibited);
    }
}

use smithay::wayland::{
    keyboard_shortcuts_inhibit::{
        KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
    },
    seat::WaylandFocus,
};

use crate::state::State;

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.wayland.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        self.wayland.shortcuts_inhibitors.push(inhibitor);
        self.sync_shortcuts_inhibitors();
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        self.wayland
            .shortcuts_inhibitors
            .retain(|held| held != &inhibitor);
    }
}

impl State {
    /// An inhibitor only applies while its own surface holds keyboard focus; a
    /// background client must not swallow the desktop's shortcuts for everyone.
    /// Follows the focus, so a game gets its shortcuts back when refocused.
    pub fn sync_shortcuts_inhibitors(&mut self) {
        let focus = self
            .wayland
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus());
        let focused = focus.as_ref().and_then(WaylandFocus::wl_surface);

        for inhibitor in &self.wayland.shortcuts_inhibitors {
            let wanted = focused.as_deref() == Some(inhibitor.wl_surface());
            if wanted != inhibitor.is_active() {
                match wanted {
                    true => inhibitor.activate(),
                    false => inhibitor.inactivate(),
                }
            }
        }
    }

    pub fn shortcuts_inhibited(&self) -> bool {
        self.wayland
            .shortcuts_inhibitors
            .iter()
            .any(KeyboardShortcutsInhibitor::is_active)
    }
}

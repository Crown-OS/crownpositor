use smithay::{
    delegate_keyboard_shortcuts_inhibit,
    wayland::{
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState,
            KeyboardShortcutsInhibitor,
        },
        seat::WaylandFocus,
    },
};

use crate::state::State;

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.wayland.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // TODO: only activate once the inhibiting surface holds keyboard focus.
        self.wayland
            .shortcuts_inhibiting_surfaces
            .insert(inhibitor.wl_surface().clone());
        inhibitor.activate();
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        self.wayland
            .shortcuts_inhibiting_surfaces
            .remove(inhibitor.wl_surface());
    }
}

impl State {
    /// An inhibitor only applies while its own surface holds keyboard focus; a
    /// background client must not swallow the desktop's shortcuts for everyone.
    pub fn shortcuts_inhibited(&self) -> bool {
        if self.wayland.shortcuts_inhibiting_surfaces.is_empty() {
            return false;
        }

        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return false;
        };
        let Some(focus) = keyboard.current_focus() else {
            return false;
        };
        let Some(surface) = focus.wl_surface() else {
            return false;
        };

        self.wayland
            .shortcuts_inhibiting_surfaces
            .contains(surface.as_ref())
    }
}

delegate_keyboard_shortcuts_inhibit!(State);

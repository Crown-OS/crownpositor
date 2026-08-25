use smithay::{
    backend::input::{Event, InputBackend, KeyState, KeyboardKeyEvent, Keycode},
    input::keyboard::{keysyms, FilterResult, KeysymHandle},
    utils::SERIAL_COUNTER,
};

use crate::{
    input::shortcuts::{Action, ModMask},
    state::State,
};

fn vt_switch_target(handle: &KeysymHandle<'_>) -> Option<i32> {
    handle.modified_syms().iter().find_map(|sym| {
        let raw = sym.raw();
        (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12)
            .contains(&raw)
            .then(|| (raw - keysyms::KEY_XF86Switch_VT_1 + 1) as i32)
    })
}

impl State {
    pub(super) fn on_keyboard_key<I: InputBackend>(&mut self, event: I::KeyboardKeyEvent) {
        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return;
        };

        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time_msec(&event);
        let mut code = event.key_code();

        if code.raw() == 9 {
            code = Keycode::new(66);
        }

        if code.raw() == 66 {
            code = Keycode::new(9);
        }

        let key_state = event.state();

        // TODO: also bypass while the session is locked, once the shell tracks
        // that — a lock screen must not be able to Super+Q out of itself.
        let bypass = self.shortcuts_inhibited();

        let action = keyboard.input::<Action, _>(
            self,
            code,
            key_state,
            serial,
            time,
            |state, modifiers, handle| {
                match key_state {
                    KeyState::Pressed => {
                        /*
                         * Disqualifies the modifier-only chord, so Super+Q does
                         * not also fire the bare-Super binding on release.
                         */
                        state.input.mod_chord_polluted = true;

                        // Ctrl+Alt+F<n>, before every inhibitor: we hold the
                        // evdev devices, so the kernel never sees this chord.
                        // It is the escape hatch out of a wedged session and no
                        // client grab may take it away.
                        if let Some(vt) = vt_switch_target(&handle) {
                            state.input.intercepted.insert(handle.raw_code());
                            return FilterResult::Intercept(Action::SwitchVt(vt));
                        }

                        if bypass {
                            return FilterResult::Forward;
                        }

                        match state.input.bindings.lookup(modifiers, &handle) {
                            Some(action) => {
                                state.input.intercepted.insert(handle.raw_code());
                                FilterResult::Intercept(action)
                            }
                            None => FilterResult::Forward,
                        }
                    }
                    KeyState::Released => {
                        // Swallow the release of an intercepted press, or the
                        // client sees a release with no matching press.
                        if state.input.intercepted.remove(&handle.raw_code()) {
                            FilterResult::Intercept(Action::None)
                        } else {
                            FilterResult::Forward
                        }
                    }
                }
            },
        );

        // Dispatch after `keyboard.input` returns, not inside the filter:
        // `handle_action` calls back into this same `KeyboardHandle`.
        if !bypass {
            self.update_mod_chord();
            if let Some(action) = self.take_mod_chord_action() {
                self.handle_action(action, serial);
                return;
            }
        }

        if let Some(action) = action {
            self.handle_action(action, serial);
        }
    }

    /// Tracks a held modifier-only chord across its press and release edges.
    fn update_mod_chord(&mut self) {
        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return;
        };
        let mask = ModMask::from_smithay(&keyboard.modifier_state());

        if mask.is_empty() {
            return;
        }

        // A new or extended modifier combination re-arms the chord.
        if self.input.mod_chord_armed != Some(mask) {
            self.input.mod_chord_armed = Some(mask);
            self.input.mod_chord_polluted = false;
        }
    }

    /// Fires on the release edge, and only if nothing else was pressed while the
    /// modifier was held.
    fn take_mod_chord_action(&mut self) -> Option<Action> {
        let keyboard = self.wayland.seat.get_keyboard()?;
        if !ModMask::from_smithay(&keyboard.modifier_state()).is_empty() {
            return None;
        }

        let armed = self.input.mod_chord_armed.take()?;
        let polluted = std::mem::take(&mut self.input.mod_chord_polluted);
        if polluted {
            return None;
        }

        self.input.bindings.lookup_mod_only(armed)
    }
}

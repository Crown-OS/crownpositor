//! Typing into the focused text input of one window.

use smithay::{
    backend::input::{InputTime, KeyState},
    input::keyboard::{FilterResult, Keycode},
    reexports::{
        wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::ZwpTextInputV3,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::SERIAL_COUNTER,
    wayland::text_input::{TextInputHandle, TextInputSeat},
};

use crate::{state::State, utils::surface::root_surface};

/// `KEY_TAB` from linux/input-event-codes.h, in xkb numbering.
const TAB_KEYCODE: u32 = 15 + 8;

impl State {
    pub(super) fn text_input_handle(&self) -> &TextInputHandle {
        self.wayland.seat.text_input()
    }

    /// Whether the seat's keyboard is on a surface of the window.
    pub(super) fn keyboard_is_in(&self, window_surface: &WlSurface) -> bool {
        self.text_input_handle()
            .focus()
            .is_some_and(|focus| root_surface(&focus) == *window_surface)
    }

    /// The active text input, when it belongs to a surface of the window.
    pub(super) fn active_text_input_in(
        &self,
        window_surface: &WlSurface,
    ) -> Option<ZwpTextInputV3> {
        let mut found = None;
        self.text_input_handle()
            .with_active_text_input(|text_input, surface| {
                if root_surface(surface) == *window_surface {
                    found = Some(text_input.clone());
                }
            });
        found
    }

    pub(super) fn press_tab(&mut self) {
        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return;
        };
        for key_state in [KeyState::Pressed, KeyState::Released] {
            keyboard.input::<(), _>(
                self,
                Keycode::new(TAB_KEYCODE),
                key_state,
                SERIAL_COUNTER.next_serial(),
                InputTime::now(),
                |_, _, _| FilterResult::Forward,
            );
        }
    }
}

/// Sends `text` to the active text input as one committed string.
pub(super) fn commit_text(handle: &TextInputHandle, text: &str) {
    handle.with_active_text_input(|text_input, _| text_input.commit_string(Some(text.to_owned())));
    handle.done(false);
}

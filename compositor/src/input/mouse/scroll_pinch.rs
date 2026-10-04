//! Alt + scroll as a pinch, for a hand on a mouse rather than a touchpad.
//!
//! The wheel is turned into the same `wp_pointer_gestures` stream a touchpad
//! pinch produces, so any client that zooms on a pinch zooms on this too,
//! anchored at the pointer. One pinch lasts while Alt is held and the pointer
//! stays put: a touchpad pinch never moves the pointer either, and the protocol
//! binds a gesture to the surface it began on.

use smithay::{
    backend::input::InputTime,
    input::pointer::{GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent},
    utils::SERIAL_COUNTER,
};

use crate::state::State;

/// Scroll distance, in logical pixels, that zooms by a factor of e. A wheel
/// notch is 15, so each notch zooms by about ten percent.
const SCROLL_PER_E_FOLD: f64 = 150.0;

const EMULATED_FINGERS: u32 = 2;

impl State {
    /// Feeds a vertical scroll into the emulated pinch. Returns `false` when Alt
    /// is not held, leaving the scroll to be delivered as a scroll.
    pub(super) fn scroll_as_pinch(&mut self, amount: f64, time: InputTime) -> bool {
        if amount == 0.0 || !self.alt_held() || self.shortcuts_inhibited() {
            return false;
        }
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return false;
        };

        let start_scale = self.input.scroll_pinch_scale.unwrap_or_else(|| {
            pointer.gesture_pinch_begin(
                self,
                &GesturePinchBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    fingers: EMULATED_FINGERS,
                },
            );
            1.0
        });
        // Scrolling up is zooming in, as with Ctrl + scroll.
        let scale = start_scale * (-amount / SCROLL_PER_E_FOLD).exp();
        self.input.scroll_pinch_scale = Some(scale);
        self.input.mod_chord_polluted = true;

        pointer.gesture_pinch_update(
            self,
            &GesturePinchUpdateEvent {
                time,
                delta: (0.0, 0.0).into(),
                scale,
                rotation: 0.0,
            },
        );
        true
    }

    pub(in crate::input) fn end_scroll_pinch(&mut self, time: InputTime) {
        if self.input.scroll_pinch_scale.take().is_none() {
            return;
        }
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        pointer.gesture_pinch_end(
            self,
            &GesturePinchEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time,
                cancelled: false,
            },
        );
    }

    pub(in crate::input) fn alt_held(&self) -> bool {
        self.wayland
            .seat
            .get_keyboard()
            .is_some_and(|keyboard| keyboard.modifier_state().alt)
    }
}

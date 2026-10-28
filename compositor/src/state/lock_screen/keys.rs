//! Input while the built-in lock is up. Keys reach it through the keyboard
//! filter, which intercepts every one; pointer events never reach a client.

use std::time::Instant;

use lockscreen::flow::{KeyInput, Response};
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Event, InputBackend, InputEvent, InputTime, PointerMotionEvent,
    },
    input::{
        keyboard::{Keysym, KeysymHandle, ModifiersState},
        pointer::MotionEvent,
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::state::State;

/// What a key press means to the lock screen, if anything.
pub fn key_input(modifiers: &ModifiersState, handle: &KeysymHandle<'_>) -> Option<KeyInput> {
    let sym = handle.modified_sym();
    if sym == Keysym::Return || sym == Keysym::KP_Enter {
        return Some(KeyInput::Submit);
    }
    if sym == Keysym::Escape {
        return Some(KeyInput::Cancel);
    }
    if sym == Keysym::BackSpace {
        return Some(if modifiers.ctrl {
            KeyInput::Clear
        } else {
            KeyInput::Backspace
        });
    }
    if modifiers.ctrl && handle.raw_latin_sym_or_raw_current_sym() == Some(Keysym::u) {
        return Some(KeyInput::Clear);
    }
    if modifiers.ctrl || modifiers.alt || modifiers.logo {
        return None;
    }
    sym.key_char()
        .filter(|c| !c.is_control())
        .map(KeyInput::Char)
}

impl State {
    pub(crate) fn on_lock_key(&mut self, input: Option<KeyInput>, caps_lock: bool) {
        let Some(lock) = self.shell.lock_screen.active_mut() else {
            return;
        };
        if lock.caps_lock != caps_lock {
            lock.caps_lock = caps_lock;
            lock.dirty = true;
            self.queue_redraw();
        }
        let Some(input) = input else {
            return;
        };
        let Some(lock) = self.shell.lock_screen.active_mut() else {
            return;
        };
        match lock.flow.key(input, Instant::now()) {
            Response::Ignored => return,
            Response::Redraw => {}
            Response::Authenticate(password) => self.submit_password(password),
        }
        self.mark_lock_dirty();
        self.schedule_lock_wake();
    }

    /// Pointer and touch input while locked: it moves the cursor and brings
    /// the prompt up, and goes no further. Returns whether `event` was taken.
    pub(crate) fn lock_screen_input<I: InputBackend>(&mut self, event: &InputEvent<I>) -> bool {
        let location = match event {
            InputEvent::Keyboard { .. }
            | InputEvent::DeviceAdded { .. }
            | InputEvent::DeviceRemoved { .. } => return false,
            InputEvent::PointerMotion { event } => {
                Some((self.input.pointer_location + event.delta(), event.time()))
            }
            InputEvent::PointerMotionAbsolute { event } => self
                .shell
                .monitor_at(self.input.pointer_location)
                .or_else(|| self.shell.focused_monitor())
                .map(|monitor| {
                    let geometry = monitor.geometry();
                    (
                        event.position_transformed(geometry.size) + geometry.loc.to_f64(),
                        event.time(),
                    )
                }),
            _ => None,
        };
        if let Some((location, time)) = location {
            self.move_locked_pointer(location, time);
        }
        let Some(lock) = self.shell.lock_screen.active_mut() else {
            return true;
        };
        if !matches!(lock.flow.pointer(Instant::now()), Response::Ignored) {
            self.mark_lock_dirty();
            self.schedule_lock_wake();
        }
        true
    }

    fn move_locked_pointer(&mut self, location: Point<f64, Logical>, time: InputTime) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };
        let previous = self.input.pointer_location;
        let location = self.clamp_to_outputs(location);
        self.input.pointer_location = location;
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
        self.queue_redraw_at(previous);
        self.queue_redraw_at(location);
    }
}

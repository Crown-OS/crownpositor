use smithay::{
    backend::input::{Event, InputBackend, KeyboardKeyEvent},
    input::keyboard::FilterResult,
    utils::SERIAL_COUNTER,
};

use crate::state::State;

impl State {
    pub(super) fn on_keyboard_key<I: InputBackend>(&mut self, event: I::KeyboardKeyEvent) {
        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return;
        };

        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time_msec(&event);

        // TODO: intercept compositor shortcuts here instead of forwarding everything.
        keyboard.input::<(), _>(
            self,
            event.key_code(),
            event.state(),
            serial,
            time,
            |_, _, _| FilterResult::Forward,
        );
    }
}

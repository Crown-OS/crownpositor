//! A seat that a protocol client drives instead of hardware: an agent cursor or a remote
//! desktop guest. Its pointer stays inside one output, and its keyboard focus only enters
//! windows on that output.
//!
//! Driven input goes straight to Smithay's pointer and keyboard handles, never through
//! [`State::process_input_event`], so no compositor shortcut fires and the user's own pointer
//! and focus are untouched.

use std::collections::HashSet;

use protocols::crownos_agent_cursor::{Axis, ButtonState, CursorEvent, CursorFrame, KeyState};
use smithay::{
    backend::input::{ButtonState as SeatButton, InputTime, KeyState as SeatKey},
    input::{
        Seat,
        keyboard::{FilterResult, KeyboardHandle, Keycode, XkbConfig},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerHandle},
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial},
};

use crate::{
    handlers::seat::KeyboardFocusTarget,
    state::State,
    utils::id::{OutputId, WindowId},
};

const XKB_KEYCODE_OFFSET: u32 = 8;
const MICROS_PER_MILLI: u64 = 1_000;
const REPEAT_DELAY_MS: i32 = 200;
const REPEAT_RATE: i32 = 65;

/// Where a driven seat may act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confinement {
    pub output: OutputId,
    pub bounds: Rectangle<i32, Logical>,
}

pub struct DrivenSeat {
    pub seat: Seat<State>,
    pub position: Point<f64, Logical>,
    pub focus: Option<WindowId>,
    held_keys: HashSet<u32>,
    held_buttons: HashSet<u32>,
}

impl State {
    /// A new `wl_seat` called `name` with a pointer at the centre of `confinement` and a
    /// keyboard; `None` when the keyboard cannot be made.
    pub(crate) fn new_driven_seat(
        &mut self,
        name: &str,
        confinement: Confinement,
    ) -> Option<DrivenSeat> {
        let display = self.common.display_handle.clone();
        let mut seat = self
            .wayland
            .seat_state
            .new_wl_seat(&display, name.to_owned());
        seat.add_pointer();
        if let Err(error) = seat.add_keyboard(XkbConfig::default(), REPEAT_DELAY_MS, REPEAT_RATE) {
            tracing::warn!(%error, seat = name, "a driven seat gets no keyboard");
            if let Some(global) = seat.global() {
                display.remove_global::<State>(global);
            }
            return None;
        }
        let bounds = confinement.bounds;
        let position = bounds.loc.to_f64() + bounds.size.to_f64().downscale(2.0).to_point();
        Some(DrivenSeat {
            seat,
            position,
            focus: None,
            held_keys: HashSet::new(),
            held_buttons: HashSet::new(),
        })
    }

    /// Applies one frame; absolute positions are relative to the confinement's top-left.
    pub(crate) fn drive_seat(
        &mut self,
        driven: &mut DrivenSeat,
        frame: CursorFrame,
        confinement: Confinement,
    ) {
        if self.shell.is_locked() {
            return;
        }
        let (Some(pointer), Some(keyboard)) =
            (driven.seat.get_pointer(), driven.seat.get_keyboard())
        else {
            return;
        };
        let bounds = confinement.bounds;
        let time = frame_time(frame.time_ms);
        for event in frame.events {
            match event {
                CursorEvent::Motion { delta } => {
                    let position = clamp(driven.position + delta, bounds);
                    self.driven_motion(driven, &pointer, position, time);
                }
                CursorEvent::MotionAbsolute { position } => {
                    let position = clamp(bounds.loc.to_f64() + position, bounds);
                    self.driven_motion(driven, &pointer, position, time);
                }
                CursorEvent::Button { button, state } => {
                    let serial = SERIAL_COUNTER.next_serial();
                    if state == ButtonState::Pressed {
                        driven.held_buttons.insert(button);
                        self.focus_under_driven(driven, &keyboard, serial, confinement);
                    } else {
                        driven.held_buttons.remove(&button);
                    }
                    pointer.button(
                        self,
                        &ButtonEvent {
                            serial,
                            time,
                            button,
                            state: seat_button(state),
                        },
                    );
                }
                CursorEvent::Axis { axis, value } => {
                    let axis = match axis {
                        Axis::VerticalScroll => smithay::backend::input::Axis::Vertical,
                        _ => smithay::backend::input::Axis::Horizontal,
                    };
                    pointer.axis(self, AxisFrame::new(time).value(axis, value));
                }
                CursorEvent::Key { key, state } => {
                    if state == KeyState::Pressed {
                        driven.held_keys.insert(key);
                        if driven.focus.is_none() {
                            let serial = SERIAL_COUNTER.next_serial();
                            self.focus_under_driven(driven, &keyboard, serial, confinement);
                        }
                    } else {
                        driven.held_keys.remove(&key);
                    }
                    self.driven_key(&keyboard, key, seat_key(state), time);
                }
            }
        }
        pointer.frame(self);
        self.queue_redraw();
    }

    fn driven_motion(
        &mut self,
        driven: &mut DrivenSeat,
        pointer: &PointerHandle<State>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        driven.position = location;
        let under = self.shell.pointer_focus_under(location);
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
    }

    fn driven_key(
        &mut self,
        keyboard: &KeyboardHandle<State>,
        key: u32,
        state: SeatKey,
        time: InputTime,
    ) {
        keyboard.input::<(), _>(
            self,
            Keycode::new(key + XKB_KEYCODE_OFFSET),
            state,
            SERIAL_COUNTER.next_serial(),
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    /// A press focuses the seat's keyboard on the window under its pointer, as a click does
    /// for the user.
    fn focus_under_driven(
        &mut self,
        driven: &mut DrivenSeat,
        keyboard: &KeyboardHandle<State>,
        serial: Serial,
        confinement: Confinement,
    ) {
        let Some((id, _)) = self.shell.window_under(driven.position) else {
            return;
        };
        if driven.focus != Some(id) {
            self.focus_driven_seat(driven, keyboard, id, serial, confinement);
        }
    }

    /// Focuses `id` on the seat's keyboard; a window outside the confinement is ignored.
    pub(crate) fn focus_driven_seat(
        &mut self,
        driven: &mut DrivenSeat,
        keyboard: &KeyboardHandle<State>,
        id: WindowId,
        serial: Serial,
        confinement: Confinement,
    ) {
        let inside = self
            .shell
            .location(id)
            .is_some_and(|location| location.output == confinement.output);
        if !inside {
            return;
        }
        let Some(window) = self.shell.tile(id).map(|tile| tile.window().clone()) else {
            return;
        };
        keyboard.set_focus(self, Some(KeyboardFocusTarget::Window(window)), serial);
        driven.focus = Some(id);
    }

    /// Releases whatever the seat held, drops its focus and removes its global.
    pub(crate) fn release_driven_seat(&mut self, driven: &mut DrivenSeat) {
        let time = frame_time(0);
        if let Some(pointer) = driven.seat.get_pointer() {
            for button in driven.held_buttons.drain() {
                pointer.button(
                    self,
                    &ButtonEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                        button,
                        state: SeatButton::Released,
                    },
                );
            }
            pointer.frame(self);
        }
        if let Some(keyboard) = driven.seat.get_keyboard() {
            let held: Vec<u32> = driven.held_keys.drain().collect();
            for key in held {
                self.driven_key(&keyboard, key, SeatKey::Released, time);
            }
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        driven.focus = None;
        if let Some(global) = driven.seat.global() {
            self.common.display_handle.remove_global::<State>(global);
        }
        self.queue_redraw();
    }
}

fn frame_time(time_ms: u32) -> InputTime {
    let millis = if time_ms == 0 {
        let now: std::time::Duration = smithay::utils::Clock::<smithay::utils::Monotonic>::new()
            .now()
            .into();
        now.as_millis() as u64
    } else {
        u64::from(time_ms)
    };
    InputTime::from_micros(millis * MICROS_PER_MILLI)
}

fn clamp(position: Point<f64, Logical>, bounds: Rectangle<i32, Logical>) -> Point<f64, Logical> {
    let bounds = bounds.to_f64();
    Point::from((
        position
            .x
            .clamp(bounds.loc.x, bounds.loc.x + bounds.size.w - 1.0),
        position
            .y
            .clamp(bounds.loc.y, bounds.loc.y + bounds.size.h - 1.0),
    ))
}

const fn seat_button(state: ButtonState) -> SeatButton {
    match state {
        ButtonState::Pressed => SeatButton::Pressed,
        _ => SeatButton::Released,
    }
}

const fn seat_key(state: KeyState) -> SeatKey {
    match state {
        KeyState::Pressed => SeatKey::Pressed,
        _ => SeatKey::Released,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_stays_on_its_output() {
        let bounds = Rectangle::new(Point::from((1920, 0)), (1280, 800).into());
        let inside = clamp(Point::from((2000.0, 400.0)), bounds);
        assert_eq!(inside, Point::from((2000.0, 400.0)));
        let left = clamp(Point::from((0.0, -50.0)), bounds);
        assert_eq!(left, Point::from((1920.0, 0.0)));
        let right = clamp(Point::from((9000.0, 9000.0)), bounds);
        assert_eq!(right, Point::from((3199.0, 799.0)));
    }
}

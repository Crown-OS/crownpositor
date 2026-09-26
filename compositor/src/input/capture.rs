//! `crownos_input_capture_v1`: handing the seat's pointer and keyboard to a
//! client once the pointer is pushed through an armed edge of the layout.
//!
//! Captured input is taken at the very top of
//! [`State::process_input_event`], before menus, the overview or shortcuts see
//! it, so nothing local reacts to input that is meant for another machine.
//! Pointer focus is dropped and a grab-like early return in the focus
//! reconcile keeps it dropped, which is also what hides the cursor from
//! every client.

use std::collections::HashSet;

use smithay::{
    backend::input::{
        Axis as SmithayAxis, ButtonState as SmithayButton, Event, InputBackend, InputEvent,
        InputTime, KeyState as SmithayKey, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
        PointerMotionEvent,
    },
    input::{
        keyboard::{FilterResult, Keycode, ModifiersState},
        pointer::{CursorImageStatus, MotionEvent},
    },
    output::Output,
    utils::{Logical, Point, SERIAL_COUNTER},
};
use wayland_server::protocol::wl_output::WlOutput;

use protocols::crownos_input::{
    CaptureAxis, CaptureButtonState, CaptureKeyState, Edge, EdgeCrossing, InputCapture,
    crossed_edge, output_point_to_layout,
};

use crate::{shell::monitor::Monitor, state::State};

/// XKB keycodes are evdev codes offset by 8.
const XKB_KEYCODE_OFFSET: u32 = 8;

#[derive(Default)]
pub struct InputCaptureState {
    /// Armed captures, most recently armed last.
    armed: Vec<(InputCapture, Edge)>,
    active: Option<ActiveCapture>,
}

struct ActiveCapture {
    capture: InputCapture,
    /// Where the pointer was when the capture began: where it goes back to
    /// when the client names no usable output.
    entered_at: Point<f64, Logical>,
    cursor: CursorImageStatus,
    /// Keys already down on entry. Their releases belong to the local seat.
    local_keys: HashSet<Keycode>,
    modifiers: Option<ModifiersState>,
}

impl InputCaptureState {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    fn arm(&mut self, capture: &InputCapture, edges: Edge) {
        self.armed
            .retain(|(armed, _)| armed != capture && armed.is_alive());
        if !edges.is_empty() {
            self.armed.push((capture.clone(), edges));
        }
    }

    fn is_active_for(&self, capture: &InputCapture) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.capture == *capture)
    }
}

impl State {
    pub fn arm_input_capture(&mut self, capture: &InputCapture, edges: Edge) {
        self.input.capture.arm(capture, edges);
    }

    pub fn release_input_capture(
        &mut self,
        capture: &InputCapture,
        output: Option<&WlOutput>,
        position: Point<f64, Logical>,
    ) {
        if !self.input.capture.is_active_for(capture) {
            return;
        }
        let target = output
            .and_then(Output::from_resource)
            .and_then(|output| self.shell.monitor(&output).map(Monitor::geometry))
            .map(|geometry| output_point_to_layout(geometry, position));
        self.end_input_capture(target);
    }

    pub fn input_capture_destroyed(&mut self, capture: &InputCapture) {
        self.input
            .capture
            .armed
            .retain(|(armed, _)| armed != capture);
        if self.input.capture.is_active_for(capture) {
            self.end_input_capture(None);
        }
    }

    /// Takes `event` if a capture wants it. Returns whether it was taken.
    pub(super) fn capture_input_event<I: InputBackend>(&mut self, event: &InputEvent<I>) -> bool {
        if self.input.capture.active.is_some() {
            return self.forward_captured(event);
        }
        match event {
            InputEvent::PointerMotion { event } => {
                self.try_enter_capture(event.delta(), event.time())
            }
            _ => false,
        }
    }

    fn try_enter_capture(&mut self, delta: Point<f64, Logical>, time: InputTime) -> bool {
        if self.input.capture.armed.is_empty() {
            return false;
        }
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return false;
        };
        if pointer.is_grabbed() || self.shell.menus.open().is_some() {
            return false;
        }

        let from = self.input.pointer_location;
        let Some(monitor) = self.shell.monitor_at(from) else {
            return false;
        };
        let output = monitor.output().clone();
        let geometry = monitor.geometry();
        let layout: Vec<_> = self
            .shell
            .monitors()
            .iter()
            .map(Monitor::geometry)
            .collect();
        let to = from + delta;

        let crossing = self
            .input
            .capture
            .armed
            .iter()
            .rev()
            .filter(|(capture, _)| capture.is_alive())
            .find_map(|(capture, edges)| {
                crossed_edge(geometry, &layout, to, *edges)
                    .map(|crossing| (capture.clone(), crossing))
            });
        let Some((capture, crossing)) = crossing else {
            return false;
        };
        self.begin_input_capture(capture, &output, crossing, time)
    }

    fn begin_input_capture(
        &mut self,
        capture: InputCapture,
        output: &Output,
        crossing: EdgeCrossing,
        time: InputTime,
    ) -> bool {
        let Some(wl_output) = capture
            .client()
            .and_then(|client| output.client_outputs(&client).next())
        else {
            tracing::debug!("input capture client has not bound the output it would enter on");
            return false;
        };
        let (Some(pointer), Some(keyboard)) = (
            self.wayland.seat.get_pointer(),
            self.wayland.seat.get_keyboard(),
        ) else {
            return false;
        };

        let location = self.input.pointer_location;
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

        let cursor = std::mem::replace(&mut self.input.cursor.status, CursorImageStatus::Hidden);
        let modifiers = keyboard.modifier_state();
        capture.entered(&wl_output, crossing.edge, crossing.position);
        send_modifiers(&capture, &modifiers);
        capture.frame(time.micros());

        self.input.capture.active = Some(ActiveCapture {
            capture,
            entered_at: location,
            cursor,
            local_keys: keyboard.pressed_keys(),
            modifiers: Some(modifiers),
        });
        self.queue_pointer_redraw();
        tracing::debug!(edge = ?crossing.edge, "input captured");
        true
    }

    fn end_input_capture(&mut self, target: Option<Point<f64, Logical>>) {
        let Some(active) = self.input.capture.active.take() else {
            return;
        };
        self.input.cursor.status = active.cursor;
        active.capture.released();
        self.warp_pointer(target.unwrap_or(active.entered_at), InputTime::now());
        self.queue_pointer_redraw();
        tracing::debug!("input released");
    }

    fn forward_captured<I: InputBackend>(&mut self, event: &InputEvent<I>) -> bool {
        let Some(capture) = self
            .input
            .capture
            .active
            .as_ref()
            .map(|active| active.capture.clone())
        else {
            return false;
        };
        if !capture.is_alive() {
            self.end_input_capture(None);
            return false;
        }

        let time = match event {
            InputEvent::PointerMotion { event } => {
                capture.motion(event.delta());
                event.time()
            }
            InputEvent::PointerButton { event } => {
                capture.button(event.button_code(), capture_button(event.state()));
                event.time()
            }
            InputEvent::PointerAxis { event } => {
                for axis in [SmithayAxis::Vertical, SmithayAxis::Horizontal] {
                    let value = event.amount(axis).unwrap_or(0.0);
                    let value120 = event.amount_v120(axis).unwrap_or(0.0) as i32;
                    if value != 0.0 || value120 != 0 {
                        capture.axis(capture_axis(axis), value, value120);
                    }
                }
                event.time()
            }
            InputEvent::Keyboard { event } => {
                if !self.capture_key::<I>(&capture, event) {
                    return false;
                }
                event.time()
            }
            InputEvent::PointerMotionAbsolute { .. }
            | InputEvent::GestureSwipeBegin { .. }
            | InputEvent::GestureSwipeUpdate { .. }
            | InputEvent::GestureSwipeEnd { .. }
            | InputEvent::GesturePinchBegin { .. }
            | InputEvent::GesturePinchUpdate { .. }
            | InputEvent::GesturePinchEnd { .. }
            | InputEvent::GestureHoldBegin { .. }
            | InputEvent::GestureHoldEnd { .. } => return true,
            _ => return false,
        };
        capture.frame(time.micros());
        true
    }

    /// Sends a key to the capture, keeping the seat's xkb state current so
    /// the modifiers it reports are right. Returns `false` for the release of
    /// a key that was already down on entry, which stays local.
    fn capture_key<I: InputBackend>(
        &mut self,
        capture: &InputCapture,
        event: &I::KeyboardKeyEvent,
    ) -> bool {
        let code = event.key_code();
        let state = event.state();
        let Some(active) = self.input.capture.active.as_mut() else {
            return false;
        };
        if active.local_keys.remove(&code) && state == SmithayKey::Released {
            return false;
        }
        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return true;
        };

        let modifiers = keyboard.input::<ModifiersState, _>(
            self,
            code,
            state,
            SERIAL_COUNTER.next_serial(),
            Event::time(event),
            |_, modifiers, _| FilterResult::Intercept(*modifiers),
        );
        capture.key(
            code.raw().saturating_sub(XKB_KEYCODE_OFFSET),
            match state {
                SmithayKey::Pressed => CaptureKeyState::Pressed,
                SmithayKey::Released => CaptureKeyState::Released,
            },
        );

        let Some(active) = self.input.capture.active.as_mut() else {
            return true;
        };
        if let Some(modifiers) = modifiers
            && active.modifiers.as_ref().map(|old| old.serialized) != Some(modifiers.serialized)
        {
            send_modifiers(capture, &modifiers);
            active.modifiers = Some(modifiers);
        }
        true
    }
}

fn send_modifiers(capture: &InputCapture, modifiers: &ModifiersState) {
    let serialized = modifiers.serialized;
    capture.modifiers(
        serialized.depressed,
        serialized.latched,
        serialized.locked,
        serialized.layout_effective,
    );
}

fn capture_button(state: SmithayButton) -> CaptureButtonState {
    match state {
        SmithayButton::Pressed => CaptureButtonState::Pressed,
        SmithayButton::Released => CaptureButtonState::Released,
    }
}

fn capture_axis(axis: SmithayAxis) -> CaptureAxis {
    match axis {
        SmithayAxis::Horizontal => CaptureAxis::HorizontalScroll,
        SmithayAxis::Vertical => CaptureAxis::VerticalScroll,
    }
}

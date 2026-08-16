use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend,
        PointerAxisEvent, PointerButtonEvent,
    },
    input::pointer::{AxisFrame, ButtonEvent, MotionEvent},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{SERIAL_COUNTER, Serial},
};

use crate::state::State;

impl State {
    pub(super) fn on_pointer_motion_absolute<I: InputBackend>(
        &mut self,
        event: I::PointerMotionAbsoluteEvent,
    ) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };
        let Some(output_geometry) = self
            .shell
            .space
            .outputs()
            .next()
            .and_then(|output| self.shell.space.output_geometry(output))
        else {
            return;
        };

        let location =
            event.position_transformed(output_geometry.size) + output_geometry.loc.to_f64();
        let under = self.shell.surface_under(location);

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
            },
        );
        pointer.frame(self);
    }

    pub(super) fn on_pointer_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let serial = SERIAL_COUNTER.next_serial();
        let state = event.state();

        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            self.focus_under_pointer(serial);
        }

        pointer.button(
            self,
            &ButtonEvent {
                button: event.button_code(),
                state,
                serial,
                time: event.time_msec(),
            },
        );
        pointer.frame(self);
    }

    pub(super) fn on_pointer_axis<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let source = event.source();
        let mut frame = AxisFrame::new(event.time_msec()).source(source);

        for axis in [Axis::Horizontal, Axis::Vertical] {
            let amount = event
                .amount(axis)
                .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);

            if amount != 0.0 {
                frame = frame.value(axis, amount);
                if let Some(v120) = event.amount_v120(axis) {
                    frame = frame.v120(axis, v120 as i32);
                }
            } else if source == AxisSource::Finger {
                frame = frame.stop(axis);
            }
        }

        pointer.axis(self, frame);
        pointer.frame(self);
    }

    /// Raises and focuses the window under the pointer, dropping focus entirely otherwise.
    fn focus_under_pointer(&mut self, serial: Serial) {
        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return;
        };
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let window = self
            .shell
            .space
            .element_under(pointer.current_location())
            .map(|(window, _)| window.clone());

        match window {
            Some(window) => {
                self.shell.space.raise_element(&window, true);
                let surface = window
                    .toplevel()
                    .map(|toplevel| toplevel.wl_surface().clone());
                keyboard.set_focus(self, surface, serial);
            }
            None => {
                for window in self.shell.space.elements() {
                    window.set_activated(false);
                }
                keyboard.set_focus(self, Option::<WlSurface>::None, serial);
            }
        }

        for window in self.shell.space.elements() {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
    }
}

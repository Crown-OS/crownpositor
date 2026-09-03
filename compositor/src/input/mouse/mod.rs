use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend,
        PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::pointer::{AxisFrame, ButtonEvent, MotionEvent},
    utils::{Logical, Point, SERIAL_COUNTER, Serial},
    wayland::{seat::WaylandFocus, shell::wlr_layer::KeyboardInteractivity},
};

use crate::{handlers::seat::PointerFocusTarget, shell::monitor::Monitor, state::State};

impl State {
    /// Absolute motion, from winit and touchscreens. Transforms against the
    /// output the pointer is on, not whichever one comes first.
    pub(super) fn on_pointer_motion_absolute<I: InputBackend>(
        &mut self,
        event: I::PointerMotionAbsoluteEvent,
    ) {
        let Some(geometry) = self
            .shell
            .monitor_at(self.input.pointer_location)
            .or_else(|| self.shell.focused_monitor())
            .map(Monitor::geometry)
        else {
            return;
        };

        let location = event.position_transformed(geometry.size) + geometry.loc.to_f64();

        self.motion(&pointer_serial_time::<I>(&event), location);
    }

    /// Relative motion, from libinput.
    pub(super) fn on_pointer_motion<I: InputBackend>(&mut self, event: I::PointerMotionEvent) {
        let location = self.clamp_to_outputs(self.input.pointer_location + event.delta());
        self.motion(&pointer_serial_time::<I>(&event), location);
    }

    fn motion(&mut self, (serial, time): &(Serial, u32), location: Point<f64, Logical>) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let previous = self.input.pointer_location;
        self.input.pointer_location = location;
        let under = self.shell.pointer_focus_under(location);

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location,
                serial: *serial,
                time: *time,
            },
        );
        pointer.frame(self);

        // The compositor draws the cursor, so a mouse move is damage like any
        // other. Both ends of the move: the output the pointer left still has
        // the old image on it. `queue` is idempotent, so a move within one
        // output costs one frame, not two.
        self.queue_redraw_at(previous);
        self.queue_redraw_at(location);
    }

    /// Keeps the pointer inside the union of the mapped outputs.
    fn clamp_to_outputs(&self, location: Point<f64, Logical>) -> Point<f64, Logical> {
        let Some(bounds) = self
            .shell
            .monitors()
            .iter()
            .map(Monitor::geometry)
            .reduce(|acc, geometry| acc.merge(geometry))
        else {
            return location;
        };

        // The far edge is exclusive; a pointer exactly on it is outside every
        // output, so nothing would be under it.
        let min_x = bounds.loc.x as f64;
        let min_y = bounds.loc.y as f64;
        let max_x = (min_x + bounds.size.w as f64 - 1.0).max(min_x);
        let max_y = (min_y + bounds.size.h as f64 - 1.0).max(min_y);

        Point::from((
            location.x.clamp(min_x, max_x),
            location.y.clamp(min_y, max_y),
        ))
    }

    pub(super) fn on_pointer_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let serial = SERIAL_COUNTER.next_serial();
        let state = event.state();

        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            self.focus_under_pointer();
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

    /// Hands focus to whatever the pointer pressed on: a window, or a layer
    /// surface that asked for the keyboard `on_demand`.
    ///
    /// Only the *model* is moved here. `update_keyboard_focus` turns that into
    /// the seat's focus and `Shell::refresh` into the activation state, so a
    /// click and a keybinding cannot disagree about who is focused.
    fn focus_under_pointer(&mut self) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let under = self
            .shell
            .pointer_focus_under(pointer.current_location())
            .map(|(target, _)| target);

        match under {
            Some(PointerFocusTarget::Window { window, .. }) => {
                if let Some(id) = window
                    .wl_surface()
                    .and_then(|surface| self.shell.window_id(&surface))
                {
                    self.shell.focus_window(id);
                }
                self.shell.focus_layer(None);
            }

            // A press on a layer surface only moves the keyboard if the surface
            // asked for it: `on_demand` means "when the user interacts with me",
            // and `none` means never. An `exclusive` surface already holds the
            // keyboard without being clicked, so it needs nothing here either.
            //
            // Window focus is deliberately left where it was. Clearing it would
            // unfocus the terminal the user is typing into every time they
            // reach for the panel or click the wallpaper.
            Some(PointerFocusTarget::LayerShell { layer, .. }) => {
                let on_demand =
                    layer.cached_state().keyboard_interactivity == KeyboardInteractivity::OnDemand;
                if on_demand {
                    self.shell.focus_layer(Some(layer));
                }
            }

            // Nothing under the pointer at all — no wallpaper, no window. There
            // is nothing to move focus to, and nothing to take it from.
            None => {}
        }

        self.update_keyboard_focus();
    }
}

fn pointer_serial_time<I: InputBackend>(event: &impl Event<I>) -> (Serial, u32) {
    (SERIAL_COUNTER.next_serial(), event.time_msec())
}

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend,
        PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::pointer::{AxisFrame, ButtonEvent, MotionEvent},
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial},
    wayland::{seat::WaylandFocus, shell::wlr_layer::KeyboardInteractivity},
};

use crate::{
    handlers::seat::PointerFocusTarget, input::decoration, shell::monitor::Monitor, state::State,
    utils::id::WindowId,
};

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

        // An open menu owns the pointer while it is inside it: highlighting
        // follows the cursor and no client hears a thing.
        if self.shell.menus.contains(location) {
            self.on_menu_motion();
            self.queue_redraw_at(previous);
            self.queue_redraw_at(location);
            return;
        }

        // And an open overview owns it the same way: the thumbnails highlight
        // under the cursor and the windows they stand for hear nothing.
        if self.overview_motion(location) || self.overview_owns_input() {
            self.queue_redraw_at(previous);
            self.queue_redraw_at(location);
            return;
        }

        let under = self.shell.pointer_focus_under(location);
        let on_frame = under
            .as_ref()
            .is_some_and(|(target, _)| target.is_decoration());
        self.track_frame_hover(on_frame);

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

    /// Keeps the pointer on a monitor.
    ///
    /// Not inside their *union*: once outputs can be arranged in two
    /// dimensions that union stops being a rectangle, and an L-shaped layout
    /// has a hole in its bounding box where nothing is drawn. A pointer that
    /// has left every output is pulled to the nearest point on the nearest
    /// one instead.
    fn clamp_to_outputs(&self, location: Point<f64, Logical>) -> Point<f64, Logical> {
        let mut nearest: Option<(f64, Point<f64, Logical>)> = None;

        for geometry in self.shell.monitors().iter().map(Monitor::geometry) {
            let clamped = clamp_to_rectangle(location, geometry);
            if clamped == location {
                return location;
            }

            let offset = clamped - location;
            let distance = offset.x * offset.x + offset.y * offset.y;
            if nearest.is_none_or(|(best, _)| distance < best) {
                nearest = Some((distance, clamped));
            }
        }

        nearest.map_or(location, |(_, clamped)| clamped)
    }

    pub(super) fn on_pointer_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let serial = SERIAL_COUNTER.next_serial();
        let state = event.state();

        // A press while a menu is open belongs to the menu: inside it, to
        // whichever row it landed on; outside, to dismissing it. Either way no
        // client sees the click, which is what makes clicking away from a menu
        // close it rather than doing two things at once.
        if state == ButtonState::Pressed
            && !pointer.is_grabbed()
            && self.shell.menus.open().is_some()
        {
            if self.shell.menus.contains(self.input.pointer_location) {
                self.on_menu_press();
            } else {
                self.dismiss_menu();
            }
            return;
        }

        // A click in the overview picks a window or a workspace, or drops a
        // window it was carrying. None of it reaches a client.
        let at = self.input.pointer_location;
        if !pointer.is_grabbed() {
            let taken = match state {
                ButtonState::Pressed => self.overview_press(at),
                ButtonState::Released => self.overview_release(at),
            };
            if taken || self.overview_owns_input() {
                return;
            }
        }

        // Which frame the click belongs to, decided before the dispatch and
        // acted on after it: a frame click can start a move grab, and smithay
        // holds the pointer's lock for the whole of `PointerHandle::button`.
        //
        // A release goes to whichever frame armed the press even when the
        // pointer has since left it — having left is exactly what cancels the
        // click, and the frame is the only thing that can say so.
        let on_left = !pointer.is_grabbed() && event.button_code() == decoration::LEFT_BUTTON;
        let frame = match state {
            ButtonState::Pressed => on_left.then(|| self.frame_under_pointer()).flatten(),
            ButtonState::Released => self.input.frame_press.map(|press| press.window),
        };

        if state == ButtonState::Pressed && !pointer.is_grabbed() && frame.is_none() {
            self.focus_under_pointer();
        }

        // Still dispatched even for a frame, so the seat's own record of which
        // buttons are down stays honest — the `Decoration` target forwards
        // nothing, so no client hears it.
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

        if let Some(window) = frame {
            self.on_frame_click(window, state, serial);
        }
    }

    /// The window whose frame the pointer is over, by id.
    fn frame_under_pointer(&self) -> Option<WindowId> {
        let (target, _) = self
            .shell
            .pointer_focus_under(self.input.pointer_location)?;
        if !target.is_decoration() {
            return None;
        }
        self.window_id_of(target.window()?)
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
            // The frame handles its own press, focus included: it has to raise
            // the window *before* deciding whether the click was a control or
            // the start of a drag.
            Some(PointerFocusTarget::Decoration { .. }) => {}

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

/// The closest point inside `rectangle`.
///
/// The far edge is exclusive: a pointer exactly on it is outside the output,
/// so nothing would be under it.
fn clamp_to_rectangle(
    location: Point<f64, Logical>,
    rectangle: Rectangle<i32, Logical>,
) -> Point<f64, Logical> {
    let min_x = rectangle.loc.x as f64;
    let min_y = rectangle.loc.y as f64;
    let max_x = (min_x + rectangle.size.w as f64 - 1.0).max(min_x);
    let max_y = (min_y + rectangle.size.h as f64 - 1.0).max(min_y);

    Point::from((
        location.x.clamp(min_x, max_x),
        location.y.clamp(min_y, max_y),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_point_inside_is_left_alone() {
        let output = rect(0, 0, 1920, 1080);
        let inside = Point::from((100.0, 100.0));
        assert_eq!(clamp_to_rectangle(inside, output), inside);
    }

    #[test]
    fn the_far_edge_is_exclusive() {
        let output = rect(0, 0, 1920, 1080);
        let clamped = clamp_to_rectangle(Point::from((5000.0, 5000.0)), output);
        assert_eq!(clamped, Point::from((1919.0, 1079.0)));
    }

    #[test]
    fn a_point_before_the_origin_lands_on_it() {
        let output = rect(1920, 0, 1920, 1080);
        let clamped = clamp_to_rectangle(Point::from((0.0, -50.0)), output);
        assert_eq!(clamped, Point::from((1920.0, 0.0)));
    }
}

//! Touchpad swipes, routed to whatever they drive.
//!
//! A four-finger horizontal swipe is *interactive*: it steers the workspace
//! viewport while the fingers move, and letting go retargets the viewport's
//! spring onto a whole page. Because the model commits the moment the fingers
//! lift, the rest of the compositor sees an ordinary workspace change while the
//! rendering catches up.
//!
//! Every other swipe waits for the release and resolves to an
//! [`Action`](crate::input::shortcuts::action::Action), down the same dispatch
//! path as a keyboard chord.

pub mod gestures;

use std::time::Duration;

use smithay::{
    backend::{
        input::{
            Event, GestureBeginEvent, GestureEndEvent, GestureSwipeUpdateEvent, InputBackend,
            UnusedEvent,
        },
        libinput::LibinputInputBackend,
    },
    reexports::input::event::gesture::{
        GestureEventCoordinates, GestureSwipeUpdateEvent as LibinputSwipeUpdate,
    },
    utils::SERIAL_COUNTER,
};

use crate::{
    input::trackpad::gestures::{Axis, Fingers},
    state::State,
};

/// The finger count that drives the workspace viewport directly.
const SWITCH_FINGERS: Fingers = Fingers::Three;

/// Unaccelerated units of finger travel that slide the viewport one workspace:
/// roughly five centimetres, at libinput's 1000 dpi normalization. Fixed, so a
/// page costs the same swipe on every monitor.
const SWIPE_DISTANCE: f64 = 500.0;

/// A swipe's motion without the pointer acceleration curve.
///
/// Acceleration is right for a cursor and wrong for direct manipulation: it
/// warps the mapping between hand and viewport, so the same physical swipe
/// travels different distances at different speeds. Backends that expose raw
/// finger motion report it here; the rest fall back to what they have.
pub trait LinearSwipe<I: InputBackend>: GestureSwipeUpdateEvent<I> {
    fn linear_delta(&self) -> (f64, f64) {
        (self.delta_x(), self.delta_y())
    }
}

impl LinearSwipe<LibinputInputBackend> for LibinputSwipeUpdate {
    fn linear_delta(&self) -> (f64, f64) {
        (self.dx_unaccelerated(), self.dy_unaccelerated())
    }
}

impl<I: InputBackend> LinearSwipe<I> for UnusedEvent {}

impl State {
    pub(super) fn on_swipe_begin<I: InputBackend>(&mut self, event: I::GestureSwipeBeginEvent) {
        self.input
            .gesture
            .begin(event.fingers(), timestamp::<I>(&event));
    }

    pub(super) fn on_swipe_update<I: InputBackend>(&mut self, event: I::GestureSwipeUpdateEvent)
    where
        I::GestureSwipeUpdateEvent: LinearSwipe<I>,
    {
        let Some(update) = self
            .input
            .gesture
            .update(event.linear_delta(), timestamp::<I>(&event))
        else {
            return;
        };

        if update.fingers != SWITCH_FINGERS || update.axis != Axis::Horizontal {
            return;
        }

        // The axis only locks partway in, so the first update to reach here is
        // where the viewport starts following the fingers.
        if !self.shell.is_swiping_workspaces() {
            self.shell.begin_workspace_swipe();
        }
        self.shell
            .update_workspace_swipe(update.delta / SWIPE_DISTANCE);
        self.queue_redraw();
    }

    pub(super) fn on_swipe_end<I: InputBackend>(&mut self, event: I::GestureSwipeEndEvent) {
        let Some(release) = self
            .input
            .gesture
            .end(event.cancelled(), timestamp::<I>(&event))
        else {
            return;
        };

        if self.shell.is_swiping_workspaces() {
            if release.cancelled {
                self.shell.cancel_workspace_swipe();
            } else {
                self.shell
                    .end_workspace_swipe(release.velocity / SWIPE_DISTANCE);
            }
            // The switch commits now and animates afterwards, so focus and
            // configures have to be brought along with it.
            self.shell.refresh();
            self.update_keyboard_focus(SERIAL_COUNTER.next_serial());
            self.queue_redraw();
            return;
        }

        if let Some(action) = release
            .gesture
            .and_then(|gesture| self.input.gesture_bindings.lookup(gesture))
        {
            self.handle_action(action, SERIAL_COUNTER.next_serial());
        }
    }
}

fn timestamp<I: InputBackend>(event: &impl Event<I>) -> Duration {
    Duration::from_micros(Event::time(event))
}

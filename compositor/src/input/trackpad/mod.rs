//! Touchpad gestures, routed to whatever they drive.
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
//!
//! Pinch is the exception: the compositor keeps none of it. It goes straight to
//! the surface under the pointer over `wp_pointer_gestures`, which is what lets
//! a browser zoom its page and a map application its map — the same division of
//! labour as scrolling, where the compositor routes and the client decides what
//! the motion means.

pub mod gestures;

use std::time::Duration;

use smithay::{
    backend::{
        input::{
            Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent,
            GestureSwipeUpdateEvent, InputBackend, UnusedEvent,
        },
        libinput::LibinputInputBackend,
    },
    input::pointer::{
        GesturePinchBeginEvent as PinchBegin, GesturePinchEndEvent as PinchEnd,
        GesturePinchUpdateEvent as PinchUpdate,
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
            self.update_keyboard_focus();
            self.queue_redraw();
            return;
        }

        if let Some(action) = release
            .gesture
            .and_then(|gesture| self.input.gesture_bindings.lookup(gesture))
        {
            self.handle_action(action);
        }
    }

    /// Pinch goes to the client, unchanged and unexamined.
    ///
    /// The three arms below are the whole of it: libinput's gesture becomes the
    /// matching `wp_pointer_gestures` event on the surface under the pointer,
    /// and the compositor forms no opinion about what a spread of the fingers
    /// ought to mean. It is the client that knows whether it has a page to zoom.
    ///
    /// The pointer handle routes to the current focus or to whatever holds the
    /// grab, so a pinch that begins over a window stays with that window even if
    /// the fingers drift — which is what the protocol requires of a gesture, and
    /// what stops a zoom from being cut in half by a moving pointer.
    pub(super) fn on_pinch_begin<I: InputBackend>(&mut self, event: I::GesturePinchBeginEvent) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        pointer.gesture_pinch_begin(
            self,
            &PinchBegin {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                fingers: event.fingers(),
            },
        );
    }

    pub(super) fn on_pinch_update<I: InputBackend>(&mut self, event: I::GesturePinchUpdateEvent) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        // Every field is passed through, rotation included: a client that only
        // wants the scale ignores the rest, and one that can turn a photo has no
        // other way to hear about it.
        pointer.gesture_pinch_update(
            self,
            &PinchUpdate {
                time: event.time_msec(),
                delta: event.delta(),
                scale: event.scale(),
                rotation: event.rotation(),
            },
        );
    }

    pub(super) fn on_pinch_end<I: InputBackend>(&mut self, event: I::GesturePinchEndEvent) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        pointer.gesture_pinch_end(
            self,
            &PinchEnd {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                cancelled: event.cancelled(),
            },
        );
    }
}

fn timestamp<I: InputBackend>(event: &impl Event<I>) -> Duration {
    Duration::from_micros(Event::time(event))
}

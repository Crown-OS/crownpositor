//! Touchpad swipes, routed to whatever they drive.
//!
//! A four-finger horizontal swipe is *interactive*: it pins the workspace
//! viewport to the fingers, and letting go hands their velocity to the spring
//! that finishes the switch. Because the model commits the moment the fingers
//! lift, the rest of the compositor sees an ordinary workspace change while the
//! rendering catches up.
//!
//! Every other swipe waits for the release and resolves to an
//! [`Action`](crate::input::shortcuts::action::Action), down the same dispatch
//! path as a keyboard chord.

pub mod gestures;

use std::time::Duration;

use smithay::{
    backend::input::{
        Event, GestureBeginEvent, GestureEndEvent, GestureSwipeUpdateEvent, InputBackend,
    },
    utils::SERIAL_COUNTER,
};

use crate::{
    input::trackpad::gestures::{Axis, Fingers},
    state::State,
};

/// The finger count that drives the workspace viewport directly.
const SWITCH_FINGERS: Fingers = Fingers::Four;

impl State {
    pub(super) fn on_swipe_begin<I: InputBackend>(&mut self, event: I::GestureSwipeBeginEvent) {
        self.input.gesture.begin(event.fingers(), timestamp::<I>(&event));
    }

    pub(super) fn on_swipe_update<I: InputBackend>(&mut self, event: I::GestureSwipeUpdateEvent) {
        let delta = event.delta();
        let Some(update) = self
            .input
            .gesture
            .update((delta.x, delta.y), timestamp::<I>(&event))
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
        self.shell.update_workspace_swipe(update.delta);
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
                self.shell.end_workspace_swipe(release.velocity);
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

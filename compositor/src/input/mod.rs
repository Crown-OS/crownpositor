mod keyboard;
pub mod libinput;
pub mod mouse;
pub mod shortcuts;
pub mod trackpad;

use smithay::backend::input::{InputBackend, InputEvent};

use crate::{input::trackpad::LinearSwipe, state::State};

impl State {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>)
    where
        I::GestureSwipeUpdateEvent: LinearSwipe<I>,
    {
        match event {
            InputEvent::Keyboard { event, .. } => self.on_keyboard_key::<I>(event),
            // Relative motion is what libinput produces; without this arm a DRM
            // session has no pointer at all.
            InputEvent::PointerMotion { event, .. } => self.on_pointer_motion::<I>(event),
            InputEvent::PointerMotionAbsolute { event, .. } => {
                self.on_pointer_motion_absolute::<I>(event)
            }
            InputEvent::PointerButton { event, .. } => self.on_pointer_button::<I>(event),
            InputEvent::PointerAxis { event, .. } => self.on_pointer_axis::<I>(event),
            InputEvent::GestureSwipeBegin { event, .. } => self.on_swipe_begin::<I>(event),
            InputEvent::GestureSwipeUpdate { event, .. } => self.on_swipe_update::<I>(event),
            InputEvent::GestureSwipeEnd { event, .. } => self.on_swipe_end::<I>(event),
            // Pinch belongs to the client: it is what a browser turns into page
            // zoom, and the compositor has no gesture of its own to spend it on.
            // These three arms hand it straight to `wp_pointer_gestures`.
            InputEvent::GesturePinchBegin { event, .. } => self.on_pinch_begin::<I>(event),
            InputEvent::GesturePinchUpdate { event, .. } => self.on_pinch_update::<I>(event),
            InputEvent::GesturePinchEnd { event, .. } => self.on_pinch_end::<I>(event),
            // TODO: hold, touch, tablet and device hotplug.
            _ => {}
        }
    }
}

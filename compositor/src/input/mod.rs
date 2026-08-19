mod keyboard;
pub mod mouse;
pub mod shortcuts;
mod trackpad;

use smithay::backend::input::{InputBackend, InputEvent};

use crate::state::State;

impl State {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
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
            // TODO: pinch, hold, touch, tablet and device hotplug.
            _ => {}
        }
    }
}

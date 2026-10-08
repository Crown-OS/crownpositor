//! The travel libinput spends recognising a gesture, handed back on its first
//! frame.
//!
//! libinput reports nothing until every finger has moved a millimetre or two,
//! and that travel is discarded rather than delivered late. On a short swipe it
//! is a large share of the whole motion, so the start feels dead. Restoring a
//! fixed amount along the direction the gesture took makes it start under the
//! fingers, the way a macOS trackpad does.

use crate::input::trackpad::gestures::UNITS_PER_MM;

/// libinput's two-finger minimum before it calls the motion a scroll.
const SCROLL_DEAD_ZONE: f64 = 1.5 * UNITS_PER_MM;
/// libinput's three-finger minimum before it calls the motion a swipe.
pub const SWIPE_DEAD_ZONE: f64 = 2.0 * UNITS_PER_MM;

/// Adds the head start to the first frame of each two-finger scroll.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScrollCatchUp {
    scrolling: bool,
}

impl ScrollCatchUp {
    /// Takes one finger-scroll frame and returns it as it should be delivered.
    /// A frame with no motion on either axis is libinput's end of the scroll.
    pub fn apply(&mut self, (x, y): (f64, f64)) -> (f64, f64) {
        if x == 0.0 && y == 0.0 {
            self.scrolling = false;
            return (x, y);
        }
        if std::mem::replace(&mut self.scrolling, true) {
            return (x, y);
        }
        // Only the dominant axis, so a scroll that starts slightly diagonal
        // does not jump sideways.
        if x.abs() >= y.abs() {
            (x + SCROLL_DEAD_ZONE.copysign(x), y)
        } else {
            (x, y + SCROLL_DEAD_ZONE.copysign(y))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_frame_of_a_scroll_is_caught_up() {
        let mut catch_up = ScrollCatchUp::default();
        assert_eq!(catch_up.apply((0.0, 4.0)), (0.0, 4.0 + SCROLL_DEAD_ZONE));
        assert_eq!(catch_up.apply((0.0, 4.0)), (0.0, 4.0));
    }

    #[test]
    fn the_head_start_follows_the_direction_of_travel() {
        let mut catch_up = ScrollCatchUp::default();
        assert_eq!(catch_up.apply((-3.0, 1.0)), (-3.0 - SCROLL_DEAD_ZONE, 1.0));
    }

    #[test]
    fn the_end_of_a_scroll_rearms_the_next_one() {
        let mut catch_up = ScrollCatchUp::default();
        catch_up.apply((0.0, 4.0));
        assert_eq!(catch_up.apply((0.0, 0.0)), (0.0, 0.0));
        assert_eq!(catch_up.apply((0.0, -2.0)), (0.0, -2.0 - SCROLL_DEAD_ZONE));
    }
}

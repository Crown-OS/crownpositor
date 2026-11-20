//! Which events one object needs to catch up with the output's brightness.

use super::Control;

/// Everything `crownos_output_brightness_v1` reports about one output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputBrightness {
    pub control: Control,
    pub level: u32,
}

impl OutputBrightness {
    /// The `control` and `level` to send after `previous`. A level always
    /// follows a control, and an answer to `set_level` always carries one.
    pub(super) fn events_since(
        self,
        previous: Option<Self>,
        answering: bool,
    ) -> (Option<Control>, Option<u32>) {
        let new_control = previous.is_none_or(|previous| previous.control != self.control);
        let new_level = previous.is_none_or(|previous| previous.level != self.level);
        (
            new_control.then_some(self.control),
            (new_control || new_level || answering).then_some(self.level),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOFTWARE_HALF: OutputBrightness = OutputBrightness {
        control: Control::Software,
        level: 5000,
    };

    #[test]
    fn a_new_object_hears_the_control_then_the_level() {
        assert_eq!(
            SOFTWARE_HALF.events_since(None, false),
            (Some(Control::Software), Some(5000))
        );
    }

    #[test]
    fn nothing_new_is_not_sent_again() {
        assert_eq!(
            SOFTWARE_HALF.events_since(Some(SOFTWARE_HALF), false),
            (None, None)
        );
    }

    #[test]
    fn the_client_that_set_the_level_always_hears_it_back() {
        assert_eq!(
            SOFTWARE_HALF.events_since(Some(SOFTWARE_HALF), true),
            (None, Some(5000))
        );
    }

    #[test]
    fn a_new_control_is_followed_by_the_level_even_when_the_level_is_the_same() {
        let backlight = OutputBrightness {
            control: Control::Backlight,
            ..SOFTWARE_HALF
        };
        assert_eq!(
            backlight.events_since(Some(SOFTWARE_HALF), false),
            (Some(Control::Backlight), Some(5000))
        );
    }

    #[test]
    fn a_new_level_is_sent_alone() {
        let brighter = OutputBrightness {
            level: 6000,
            ..SOFTWARE_HALF
        };
        assert_eq!(
            brighter.events_since(Some(SOFTWARE_HALF), false),
            (None, Some(6000))
        );
    }
}

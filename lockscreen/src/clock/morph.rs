//! A digit changing: the old one blurs away as the new one blurs in.

use std::time::Duration;

use crownos_config::schema::Easing;

use crate::transition::{Curve, LayerStyle};

pub const MORPH_DURATION: Duration = Duration::from_millis(520);
/// Logical pixels, at the height of the change.
pub const MORPH_BLUR: f32 = 14.0;

/// The share of the change the old digit takes to go.
const LEAVE_SPAN: f32 = 0.6;
/// When in the change the new digit starts to arrive.
const ENTER_START: f32 = 0.25;
/// How much the old digit grows as it goes, and the new one as it comes.
const SWELL: f32 = 0.06;

/// The leaving digit and the arriving one, `elapsed` into a change.
pub fn morph(elapsed: Duration) -> (LayerStyle, LayerStyle) {
    let time = elapsed.as_secs_f32() / MORPH_DURATION.as_secs_f32();
    let gone = Easing::EaseOut.ease(time / LEAVE_SPAN);
    let here = Easing::EaseInOut.ease((time - ENTER_START) / (1.0 - ENTER_START));
    let leaving = LayerStyle {
        opacity: 1.0 - gone,
        blur: MORPH_BLUR * gone,
        scale: 1.0 + SWELL * gone,
        ..LayerStyle::REST
    };
    let entering = LayerStyle {
        opacity: here,
        blur: MORPH_BLUR * (1.0 - here),
        scale: 1.0 - SWELL * (1.0 - here),
        ..LayerStyle::REST
    };
    (leaving, entering)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_starts_on_the_old_digit_and_ends_on_the_new() {
        let (leaving, entering) = morph(Duration::ZERO);
        assert_eq!(leaving, LayerStyle::REST);
        assert!(!entering.is_visible());
        let (leaving, entering) = morph(MORPH_DURATION);
        assert!(!leaving.is_visible());
        assert_eq!(entering, LayerStyle::REST);
    }

    #[test]
    fn both_digits_are_blurred_while_they_cross() {
        let (leaving, entering) = morph(MORPH_DURATION / 2);
        assert!(leaving.blur > 0.0 && entering.blur > 0.0);
        assert!(leaving.is_visible() || entering.is_visible());
    }
}

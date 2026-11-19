//! The lock screen's one choreography: the clock and the
//! background's subject rise in from below the bottom edge over the sharp
//! desktop, then the background floods out from behind the subject to the
//! screen's edges; the clock gives way to the
//! prompt by rising and receding into a blurred, dimmed wallpaper; and on
//! unlock everything swells a little and dissolves into the desktop.

use super::{Frame, LayerStyle};

/// Logical pixels the clock rises as the prompt comes up: 6 % of a typical
/// 1080-pixel-tall output.
const REVEAL_CLOCK_RISE: f32 = 0.06 * 1080.0;
/// What the clock shrinks to and how far it blurs as it gives way.
const REVEAL_CLOCK_SCALE: f32 = 0.9;
const REVEAL_CLOCK_BLUR: f32 = 20.0;
/// Logical pixels below its place the prompt rises from, and the size it
/// grows from.
const REVEAL_PROMPT_DROP: f32 = 40.0;
const REVEAL_PROMPT_SCALE: f32 = 0.96;

/// Logical pixels: the sigma of the gaussian the wallpaper is blurred by
/// behind the prompt.
pub const PROMPT_BLUR: f32 = 40.0;
/// How much darker the wallpaper gets behind the prompt, on top of the
/// lock's own dim.
pub const PROMPT_DIM: f32 = 0.25;

/// What the clock and prompt swell to as they leave, and how far they blur.
const DISMISS_SCALE: f32 = 1.06;
const DISMISS_BLUR: f32 = 12.0;
/// What the wallpaper zooms to as it fades off the desktop.
const DISMISS_WALLPAPER_SCALE: f32 = 1.08;

/// Logical pixels: the widest blur any one moment gives the clock or prompt
/// layer, the padding a host leaves around their sprites for it.
pub const LAYER_BLUR_REACH: f32 = REVEAL_CLOCK_BLUR.max(DISMISS_BLUR);

/// How far along each moment the lock is: 0 before it starts, 1 once it has
/// landed, and a touch past 1 while a spring overshoots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moments {
    /// Locking: the clock and subject rising in over the desktop.
    pub rise: f32,
    /// Locking, once they have: the background flooding out behind them.
    pub flood: f32,
    /// The clock giving way to the password prompt.
    pub reveal: f32,
    /// Unlocking: the lock dissolving into the desktop.
    pub dismiss: f32,
}

impl Moments {
    /// Locked, showing the clock.
    pub const CLOCK: Self = Self {
        rise: 1.0,
        flood: 1.0,
        reveal: 0.0,
        dismiss: 0.0,
    };

    /// Locked, showing the password prompt.
    pub const PROMPT: Self = Self {
        reveal: 1.0,
        ..Self::CLOCK
    };

    /// Everything drawn at these moments, the password field `shake` logical
    /// pixels to the side.
    pub fn frame(self, shake: f32) -> Frame {
        Frame {
            clock: self.clock(),
            prompt: self.prompt(),
            wallpaper: self.wallpaper(),
            subject: self.subject(),
            rise: self.rise,
            flood: self.flood,
            dim: PROMPT_DIM * shown(self.reveal),
            shake,
        }
    }

    /// Whole and sharp as it rises in: [`Frame::risen`] places it.
    fn clock(self) -> LayerStyle {
        let reveal = self.reveal;
        let giving_way = LayerStyle {
            opacity: 1.0 - shown(reveal),
            offset: [0.0, -REVEAL_CLOCK_RISE * reveal],
            scale: mix(1.0, REVEAL_CLOCK_SCALE, reveal),
            blur: REVEAL_CLOCK_BLUR * shown(reveal),
        };
        giving_way.then(leaving(self.dismiss))
    }

    fn prompt(self) -> LayerStyle {
        let reveal = self.reveal;
        if reveal <= 0.0 {
            return LayerStyle::HIDDEN;
        }
        let rising = LayerStyle {
            opacity: shown(reveal),
            offset: [0.0, REVEAL_PROMPT_DROP * (1.0 - reveal)],
            scale: mix(REVEAL_PROMPT_SCALE, 1.0, reveal),
            ..LayerStyle::REST
        };
        rising.then(leaving(self.dismiss))
    }

    /// Never zoomed on the way in, so the subject rises into its place in
    /// the picture and stays there as the picture floods out around it.
    fn wallpaper(self) -> LayerStyle {
        let dismiss = self.dismiss;
        LayerStyle {
            opacity: 1.0 - shown(dismiss),
            offset: [0.0; 2],
            scale: mix(1.0, DISMISS_WALLPAPER_SCALE, dismiss),
            blur: PROMPT_BLUR * shown(self.reveal),
        }
    }

    /// Framed with the wallpaper but sharp while it floods, and shown in
    /// full from the start: rising in is its entrance, which
    /// [`Frame::risen`] places.
    fn subject(self) -> LayerStyle {
        let wallpaper = self.wallpaper();
        LayerStyle {
            opacity: wallpaper.opacity * (1.0 - shown(self.reveal)) * (1.0 - shown(self.dismiss)),
            blur: PROMPT_BLUR * shown(self.reveal),
            ..wallpaper
        }
    }
}

/// The clock and prompt on their way out.
fn leaving(dismiss: f32) -> LayerStyle {
    LayerStyle {
        opacity: 1.0 - shown(dismiss),
        scale: mix(1.0, DISMISS_SCALE, dismiss),
        blur: DISMISS_BLUR * shown(dismiss),
        ..LayerStyle::REST
    }
}

/// `progress` held to `0..=1`, for what must not overshoot: opacity, blur.
fn shown(progress: f32) -> f32 {
    progress.clamp(0.0, 1.0)
}

/// From `from` at 0 to `to` at 1, exactly at both ends, overshooting with
/// `progress`.
fn mix(from: f32, to: f32, progress: f32) -> f32 {
    from * (1.0 - progress) + to * progress
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(arrival: f32, reveal: f32, dismiss: f32) -> Frame {
        Moments {
            rise: arrival,
            flood: arrival,
            reveal,
            dismiss,
        }
        .frame(0.0)
    }

    fn arriving(rise: f32, flood: f32) -> Frame {
        Moments {
            rise,
            flood,
            ..Moments::CLOCK
        }
        .frame(0.0)
    }

    #[test]
    fn the_resting_clock_is_untouched_and_sharp() {
        let frame = Moments::CLOCK.frame(0.0);
        assert_eq!(frame.clock, LayerStyle::REST);
        assert!(!frame.prompt.is_visible());
        assert_eq!(frame.wallpaper, LayerStyle::REST);
        assert_eq!(frame.subject, LayerStyle::REST);
        assert_eq!(frame.flood, 1.0);
        assert_eq!(frame.risen(1000.0), frame);
        assert_eq!(frame.dim, 0.0);
    }

    #[test]
    fn the_prompt_rests_over_a_blurred_dimmed_wallpaper() {
        let frame = Moments::PROMPT.frame(0.0);
        assert_eq!(frame.prompt, LayerStyle::REST);
        assert!(!frame.clock.is_visible());
        assert_eq!(frame.wallpaper.blur, PROMPT_BLUR);
        assert_eq!(frame.wallpaper.opacity, 1.0);
        assert_eq!(frame.dim, PROMPT_DIM);
    }

    #[test]
    fn the_clock_and_subject_rise_in_whole_and_sharp_from_below_the_output() {
        let start = arriving(0.0, 0.0).risen(900.0);
        for layer in [start.clock, start.subject] {
            assert_eq!(layer.offset[1], 900.0);
            assert_eq!((layer.opacity, layer.scale, layer.blur), (1.0, 1.0, 0.0));
        }
        assert_eq!(start.wallpaper.scale, 1.0);
        let halfway = arriving(0.5, 0.0).risen(900.0);
        assert_eq!(halfway.clock.offset, halfway.subject.offset);
        assert!(
            arriving(1.01, 0.0).risen(900.0).subject.offset[1] < 0.0,
            "they overshoot"
        );
        assert!(!at(1.0, 1.0, 0.0).subject.is_visible());
    }

    #[test]
    fn nothing_blurs_or_darkens_the_desktop_as_the_lock_arrives() {
        for (rise, flood) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (1.0, 0.5)] {
            assert_eq!(arriving(rise, flood).wallpaper.blur, 0.0);
        }
        assert_eq!(arriving(1.0, 0.0).darkening(0.6), 0.0);
    }

    #[test]
    fn an_overshoot_never_shows_through() {
        let frame = at(1.01, 1.01, 0.0);
        assert!(frame.clock.opacity >= 0.0 && frame.clock.blur >= 0.0);
        assert!(frame.wallpaper.opacity <= 1.0);
        let gone = at(1.0, 1.0, 1.01);
        assert!(!gone.clock.is_visible() && !gone.prompt.is_visible());
        assert!(!gone.wallpaper.is_visible());
    }

    #[test]
    fn a_finished_dismiss_leaves_only_the_desktop() {
        for reveal in [0.0, 1.0] {
            let frame = at(1.0, reveal, 1.0);
            assert_eq!(frame.clock.opacity, 0.0);
            assert_eq!(frame.prompt.opacity, 0.0);
            assert_eq!(frame.wallpaper.opacity, 0.0);
            assert_eq!(frame.darkening(0.6), 0.0);
        }
        assert_eq!(at(1.0, 0.0, 1.0).clock.scale, DISMISS_SCALE);
    }
}

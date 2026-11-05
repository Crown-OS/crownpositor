//! The colours a window frame is drawn in.
//!
//! Resolved once per frame from the theme rather than per window, and handed to
//! the shader already premultiplied — that is the form it composites in, and
//! multiplying in GLSL would do it once per pixel instead of once per frame.

use config::Appearance;
use protocols::crownos_android_shell::TitleColor;

/// Straight (non-premultiplied) RGBA, the form these are written in.
type Rgba = [f32; 4];

/// Premultiplies, which is what every shader here expects.
fn premultiply([red, green, blue, alpha]: Rgba) -> Rgba {
    [red * alpha, green * alpha, blue * alpha, alpha]
}

/// Scales a premultiplied colour's coverage without changing its hue.
fn fade(color: Rgba, factor: f32) -> Rgba {
    color.map(|channel| channel * factor)
}

/// Nothing at all, premultiplied.
const CLEAR: Rgba = [0.0; 4];

/// How much of its colour an unfocused window's frame keeps. Enough to stay
/// legible, little enough that the focused window is obvious at a glance.
const UNFOCUSED: f32 = 0.62;

/// How much darker an app-coloured titlebar gets towards its bottom edge.
const TINT_SHADE: f32 = 0.9;

/// The relative luminance at which black and white text contrast equally with
/// the background: `(L + 0.05) / 0.05 == 1.05 / (L + 0.05)`.
const CONTRAST_CROSSOVER: f32 = 0.179;

/// Which theme the desktop is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn of(appearance: &Appearance) -> Self {
        if appearance.dark_mode {
            Self::Dark
        } else {
            Self::Light
        }
    }
}

/// The text and glyph colour that reads on a background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Light,
    Dark,
}

impl Ink {
    /// Dark on a light background and light on a dark one, by the
    /// background's relative luminance.
    pub fn on(background: TitleColor) -> Self {
        let [red, green, blue, _] = background.rgba().map(linear);
        let luminance = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
        if luminance > CONTRAST_CROSSOVER {
            Self::Dark
        } else {
            Self::Light
        }
    }
}

/// An sRGB channel in linear light.
fn linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// One theme's frame colours, premultiplied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramePalette {
    pub tint_top: Rgba,
    pub tint_bottom: Rgba,
    pub highlight: Rgba,
    pub control_fill: Rgba,
    pub control_glyph: Rgba,
    pub border: Rgba,
    /// Straight, not premultiplied: the title is rasterised on the CPU into a
    /// buffer the renderer premultiplies itself.
    pub label: Rgba,
    /// Straight: the tint the frame's glass is mixed with, which is what the
    /// panel's gradient becomes when the frame stands on glass.
    pub sheet: Rgba,
}

impl FramePalette {
    pub fn new(appearance: &Appearance, focused: bool) -> Self {
        Self::themed(Theme::of(appearance)).focus(focused)
    }

    /// An Android app's frame: its own colour on the panel and the glass, and
    /// the title and controls in whichever ink reads on it.
    pub fn tinted(theme: Theme, focused: bool, color: TitleColor) -> Self {
        Self::themed(theme).tint(color).focus(focused)
    }

    /// The frame over glass reads as a sheet of the desktop's own material, so
    /// both themes are a translucent wash rather than a solid fill — the blur
    /// underneath is what gives it its colour.
    fn themed(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self {
                tint_top: premultiply([0.20, 0.20, 0.22, 0.88]),
                tint_bottom: premultiply([0.13, 0.13, 0.15, 0.72]),
                highlight: premultiply([1.0, 1.0, 1.0, 0.10]),
                control_fill: premultiply([1.0, 1.0, 1.0, 0.16]),
                control_glyph: premultiply([1.0, 1.0, 1.0, 0.82]),
                border: premultiply([1.0, 1.0, 1.0, 0.18]),
                label: [0.94, 0.94, 0.96, 1.0],
                sheet: [0.17, 0.17, 0.19, 0.80],
            },
            Theme::Light => Self {
                tint_top: premultiply([0.96, 0.96, 0.97, 0.70]),
                tint_bottom: premultiply([0.87, 0.87, 0.89, 0.76]),
                highlight: premultiply([1.0, 1.0, 1.0, 0.55]),
                control_fill: premultiply([0.79, 0.79, 0.81, 0.92]),
                control_glyph: premultiply([0.22, 0.22, 0.24, 1.0]),
                border: premultiply([1.0, 1.0, 1.0, 0.45]),
                label: [0.11, 0.11, 0.13, 1.0],
                sheet: [0.92, 0.92, 0.93, 0.73],
            },
        }
    }

    fn tint(self, color: TitleColor) -> Self {
        let [red, green, blue, alpha] = color.rgba();
        let (label, control_fill, control_glyph) = match Ink::on(color) {
            Ink::Dark => (
                [0.11, 0.11, 0.13, 1.0],
                [0.0, 0.0, 0.0, 0.14],
                [0.11, 0.11, 0.13, 0.90],
            ),
            Ink::Light => (
                [0.98, 0.98, 1.0, 1.0],
                [1.0, 1.0, 1.0, 0.22],
                [1.0, 1.0, 1.0, 0.92],
            ),
        };
        let shade = |channel: f32| channel * TINT_SHADE;
        Self {
            tint_top: premultiply([red, green, blue, alpha]),
            tint_bottom: premultiply([shade(red), shade(green), shade(blue), alpha]),
            control_fill: premultiply(control_fill),
            control_glyph: premultiply(control_glyph),
            label,
            sheet: [red, green, blue, alpha],
            ..self
        }
    }

    fn focus(self, focused: bool) -> Self {
        if focused {
            return self;
        }
        Self {
            tint_top: fade(self.tint_top, UNFOCUSED),
            tint_bottom: fade(self.tint_bottom, UNFOCUSED),
            highlight: fade(self.highlight, UNFOCUSED),
            control_fill: fade(self.control_fill, UNFOCUSED),
            control_glyph: fade(self.control_glyph, UNFOCUSED),
            border: fade(self.border, UNFOCUSED),
            label: [
                self.label[0],
                self.label[1],
                self.label[2],
                self.label[3] * UNFOCUSED,
            ],
            sheet: [
                self.sheet[0],
                self.sheet[1],
                self.sheet[2],
                self.sheet[3] * UNFOCUSED,
            ],
        }
    }

    /// What the panel still draws once the glass under it carries the tint:
    /// the controls alone. The glass's own rim is the highlight, traced round
    /// the corners where a straight line would stop against them.
    pub fn over_glass(self) -> Self {
        Self {
            tint_top: CLEAR,
            tint_bottom: CLEAR,
            highlight: CLEAR,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appearance(dark: bool) -> Appearance {
        Appearance {
            dark_mode: dark,
            ..Appearance::default()
        }
    }

    /// A premultiplied colour can never have a channel above its own alpha, and
    /// a shader fed one composites haloes around every edge.
    #[test]
    fn every_colour_is_premultiplied() {
        for dark in [true, false] {
            for focused in [true, false] {
                let palette = FramePalette::new(&appearance(dark), focused);
                for color in [
                    palette.tint_top,
                    palette.tint_bottom,
                    palette.highlight,
                    palette.control_fill,
                    palette.control_glyph,
                    palette.border,
                ] {
                    let alpha = color[3];
                    assert!(
                        color[..3].iter().all(|channel| *channel <= alpha + 1e-6),
                        "{color:?} exceeds its own alpha"
                    );
                }
            }
        }
    }

    fn color(argb: u32) -> TitleColor {
        TitleColor::new(argb).expect("an opaque colour")
    }

    #[test]
    fn title_text_reads_on_the_app_colour() {
        for light in [0xffff_ffff, 0xffff_eb3b, 0xff8b_c34a, 0xff00_bcd4] {
            assert_eq!(Ink::on(color(light)), Ink::Dark, "{light:#x}");
        }
        for dark in [0xff00_0000, 0xff1a_237e, 0xffb7_1c1c, 0xff6a_1b9a] {
            assert_eq!(Ink::on(color(dark)), Ink::Light, "{dark:#x}");
        }
    }

    #[test]
    fn a_tinted_frame_takes_the_app_colour_and_readable_text() {
        let navy = color(0xff1a_237e);
        let palette = FramePalette::tinted(Theme::Light, true, navy);
        assert_eq!(palette.sheet, navy.rgba());
        assert_eq!(palette.tint_top, premultiply(navy.rgba()));
        assert!(palette.label[..3].iter().all(|channel| *channel > 0.9));

        let unfocused = FramePalette::tinted(Theme::Light, false, navy);
        assert!(unfocused.tint_top[3] < palette.tint_top[3]);
    }

    #[test]
    fn the_two_themes_differ() {
        assert_ne!(
            FramePalette::new(&appearance(true), true).tint_top,
            FramePalette::new(&appearance(false), true).tint_top
        );
    }

    #[test]
    fn losing_focus_only_fades() {
        let focused = FramePalette::new(&appearance(true), true);
        let unfocused = FramePalette::new(&appearance(true), false);

        assert!(unfocused.tint_top[3] < focused.tint_top[3]);
        // Same hue: every channel scaled by the same factor.
        let ratio = unfocused.tint_top[3] / focused.tint_top[3];
        for index in 0..3 {
            let scaled = focused.tint_top[index] * ratio;
            assert!((unfocused.tint_top[index] - scaled).abs() < 1e-6);
        }
    }
}

//! The colours a window frame is drawn in.
//!
//! Resolved once per frame from the theme rather than per window, and handed to
//! the shader already premultiplied — that is the form it composites in, and
//! multiplying in GLSL would do it once per pixel instead of once per frame.

use config::Appearance;

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

/// How much of its colour an unfocused window's frame keeps. Enough to stay
/// legible, little enough that the focused window is obvious at a glance.
const UNFOCUSED: f32 = 0.62;

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
}

impl FramePalette {
    /// The frame over glass reads as a sheet of the desktop's own material, so
    /// both themes are a translucent wash rather than a solid fill — the blur
    /// underneath is what gives it its colour.
    pub fn new(appearance: &Appearance, focused: bool) -> Self {
        let palette = if appearance.dark_mode {
            Self {
                tint_top: premultiply([0.20, 0.20, 0.22, 0.88]),
                tint_bottom: premultiply([0.13, 0.13, 0.15, 0.72]),
                highlight: premultiply([1.0, 1.0, 1.0, 0.10]),
                control_fill: premultiply([1.0, 1.0, 1.0, 0.16]),
                control_glyph: premultiply([1.0, 1.0, 1.0, 0.82]),
                border: premultiply([1.0, 1.0, 1.0, 0.18]),
                label: [0.94, 0.94, 0.96, 1.0],
            }
        } else {
            Self {
                tint_top: premultiply([0.96, 0.96, 0.97, 0.70]),
                tint_bottom: premultiply([0.87, 0.87, 0.89, 0.76]),
                highlight: premultiply([1.0, 1.0, 1.0, 0.55]),
                control_fill: premultiply([0.79, 0.79, 0.81, 0.92]),
                control_glyph: premultiply([0.22, 0.22, 0.24, 1.0]),
                border: premultiply([1.0, 1.0, 1.0, 0.45]),
                label: [0.11, 0.11, 0.13, 1.0],
            }
        };

        if focused {
            palette
        } else {
            Self {
                tint_top: fade(palette.tint_top, UNFOCUSED),
                tint_bottom: fade(palette.tint_bottom, UNFOCUSED),
                highlight: fade(palette.highlight, UNFOCUSED),
                control_fill: fade(palette.control_fill, UNFOCUSED),
                control_glyph: fade(palette.control_glyph, UNFOCUSED),
                border: fade(palette.border, UNFOCUSED),
                label: [
                    palette.label[0],
                    palette.label[1],
                    palette.label[2],
                    palette.label[3] * UNFOCUSED,
                ],
            }
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

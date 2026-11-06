use lockscene::AssetId;

use super::stroked::{self, StrokedFont};
use crate::{clock::Glass, palette::Rgba};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontFamily {
    Sans(String),
    Mono(String),
}

/// One run of text, laid out within its sprite's width.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    /// Logical pixels.
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
    pub family: FontFamily,
    pub color: Rgba,
    pub centered: bool,
    pub single_line: bool,
}

/// One character of the clock, drawn from its outline so it can stand
/// taller than the font draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    pub ch: char,
    /// Logical pixels.
    pub size: f32,
    /// From hairline, 100, to heavy, 900.
    pub weight: f32,
    /// Drawn by the lock screen itself; a character it has no digit for
    /// falls back to the system's sans.
    pub font: StrokedFont,
    /// How much taller than the font draws it.
    pub stretch: f32,
    pub paint: GlyphPaint,
}

/// Of the size: how far round digits rise past the cap height.
const OVERSHOOT: f32 = 0.04;
/// Of the glyph's height, so a taller clock casts a proportionally softer
/// shadow: how far the shadow drops, and the radius of each blur pass,
/// about the gaussian sigma three passes make. ColorOS's references show no
/// measurable shadow (under 2% darkening at any sigma up to 0.04 of the
/// height), so this is the faint, wide one its reviews call "a subtle shadow
/// at the base", not a hard drop.
pub(crate) const SHADOW_DROP: f32 = 0.008;
pub(crate) const SHADOW_SPREAD: f32 = 0.025;
/// Three box-blur passes spread a shadow this many radii.
const BLUR_PASSES: f32 = 3.0;

impl GlyphRun {
    /// Logical pixels the glyph draws past its box.
    pub fn reach(&self) -> f32 {
        let overshoot = OVERSHOOT * self.size * self.stretch;
        match self.paint {
            GlyphPaint::Shadow { .. } => {
                overshoot
                    + (SHADOW_DROP + BLUR_PASSES * SHADOW_SPREAD)
                        * stroked::height(self.size, self.stretch)
            }
            GlyphPaint::Glass { .. } | GlyphPaint::Coverage => overshoot,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphPaint {
    /// The digit itself, [`Glass`] of `whiteness`, from frosted, 0, to solid
    /// white, 1. A host that sees the background paints the sprite as
    /// [`Coverage`](Self::Coverage) and draws the glass over the background
    /// itself; painted as is, it is the white alone, as a thumbnail shows it.
    Glass { whiteness: f32 },
    /// Plain white coverage, for a host that fills the glyph itself.
    Coverage,
    /// Black at `alpha`, dropped and blurred.
    Shadow { alpha: f32 },
}

impl GlyphPaint {
    /// The glyph itself, as opposed to what is drawn under it.
    pub fn is_face(&self) -> bool {
        matches!(self, Self::Glass { .. })
    }

    /// The glass a face is drawn with.
    pub fn glass(&self) -> Option<Glass> {
        match *self {
            Self::Glass { whiteness } => Some(Glass::at(whiteness)),
            Self::Coverage | Self::Shadow { .. } => None,
        }
    }
}

/// What one sprite shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text(TextRun),
    Glyph(GlyphRun),
    /// A Lucide icon's SVG body.
    Icon {
        body: &'static str,
        color: Rgba,
    },
    Image {
        asset: AssetId,
        radius: f32,
    },
    Panel {
        fill: Option<Rgba>,
        border: Option<(f32, Rgba)>,
        radius: f32,
    },
    Divider {
        color: Rgba,
    },
    Progress {
        value: f32,
        track: Rgba,
        fill: Rgba,
    },
    /// The password field: one dot per character typed, or a hint.
    Field {
        dots: usize,
        busy: bool,
        hint: TextRun,
        fill: Rgba,
        border: Rgba,
        dot: Rgba,
    },
    Avatar {
        initial: TextRun,
        fill: [Rgba; 2],
    },
}

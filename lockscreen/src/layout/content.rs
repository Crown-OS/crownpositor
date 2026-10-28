use lockscene::AssetId;

use crate::palette::Rgba;

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
    pub weight: u16,
    pub family: FontFamily,
    /// How much taller than the font draws it.
    pub stretch: f32,
    pub paint: GlyphPaint,
}

/// Of the size: how far round digits rise past the cap height.
const OVERSHOOT: f32 = 0.04;
/// Of the size: how far the shadow drops, and how wide it spreads.
pub(crate) const SHADOW_DROP: f32 = 0.02;
pub(crate) const SHADOW_SPREAD: f32 = 0.035;
/// Three box-blur passes spread a shadow this many radii.
const BLUR_PASSES: f32 = 3.0;

impl GlyphRun {
    /// Logical pixels the glyph draws past its box.
    pub fn reach(&self) -> f32 {
        let overshoot = OVERSHOOT * self.size * self.stretch;
        match self.paint {
            GlyphPaint::Shadow { .. } => {
                overshoot + (SHADOW_DROP + BLUR_PASSES * SHADOW_SPREAD) * self.size
            }
            GlyphPaint::Face { .. } | GlyphPaint::Coverage => overshoot,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphPaint {
    /// Fades from `top` at the cap height to `bottom` on the baseline.
    Face { top: Rgba, bottom: Rgba },
    /// Plain white coverage, for a host that fills the glyph itself.
    Coverage,
    /// Black at `alpha`, dropped and blurred.
    Shadow { alpha: f32 },
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

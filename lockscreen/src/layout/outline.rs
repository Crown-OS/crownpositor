use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, Command, FeatureTag, FontFeatures, Metrics, Shaping, Weight,
};
use tiny_skia::{Path, PathBuilder};

use super::{GlyphRun, Shaper, text::family_of};

/// Of the ascent, the cap height of a font that does not say.
const CAP_OF_ASCENT: f32 = 0.72;
const TABULAR_FIGURES: FeatureTag = FeatureTag::new(b"tnum");

/// A glyph's outline at its run's size, y up from its origin on the baseline,
/// not yet stretched.
pub(crate) struct GlyphShape {
    /// `None` for a glyph that draws nothing, like a space.
    pub path: Option<Path>,
    pub advance: f32,
    pub cap_height: f32,
}

impl GlyphShape {
    /// The box the glyph stands in: its advance, and its cap height stretched.
    pub fn size(&self, stretch: f32) -> [f32; 2] {
        [self.advance.ceil(), (self.cap_height * stretch).ceil()]
    }
}

impl Shaper {
    pub fn glyph(&mut self, run: &GlyphRun) -> Option<GlyphShape> {
        let mut features = FontFeatures::new();
        features.enable(TABULAR_FIGURES);
        let attrs = Attrs::new()
            .family(family_of(&run.family))
            .weight(Weight(run.weight))
            .font_features(features)
            .cache_key_flags(CacheKeyFlags::DISABLE_HINTING);
        let mut utf8 = [0; 4];
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(run.size, run.size));
        let mut borrowed = buffer.borrow_with(&mut self.fonts);
        borrowed.set_text(
            run.ch.encode_utf8(&mut utf8),
            &attrs,
            Shaping::Advanced,
            None,
        );
        borrowed.shape_until_scroll(false);
        let glyph = buffer
            .layout_runs()
            .find_map(|line| line.glyphs.first())?
            .clone();

        let font = self.fonts.get_font(glyph.font_id, glyph.font_weight)?;
        let metrics = font.metrics();
        let cap = metrics
            .cap_height
            .filter(|cap| *cap > 0.0)
            .unwrap_or(metrics.ascent * CAP_OF_ASCENT);
        let cap_height = cap / f32::from(metrics.units_per_em.max(1)) * run.size;

        let key = glyph.physical((0.0, 0.0), 1.0).cache_key;
        let path = self
            .glyphs
            .get_outline_commands(&mut self.fonts, key)
            .and_then(path_of);
        Some(GlyphShape {
            path,
            advance: glyph.w,
            cap_height,
        })
    }
}

fn path_of(commands: &[Command]) -> Option<Path> {
    let mut path = PathBuilder::new();
    for command in commands {
        match *command {
            Command::MoveTo(to) => path.move_to(to.x, to.y),
            Command::LineTo(to) => path.line_to(to.x, to.y),
            Command::QuadTo(control, to) => path.quad_to(control.x, control.y, to.x, to.y),
            Command::CurveTo(first, second, to) => {
                path.cubic_to(first.x, first.y, second.x, second.y, to.x, to.y)
            }
            Command::Close => path.close(),
        }
    }
    path.finish()
}

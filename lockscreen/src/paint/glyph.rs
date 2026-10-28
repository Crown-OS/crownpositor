use tiny_skia::{Pixmap, Point, Transform};

use crate::{
    layout::{GlyphPaint, GlyphRun, SHADOW_DROP, SHADOW_SPREAD, Shaper},
    util::blur::box_blur,
};

use super::shape::{fill, gradient_fill};

const WHITE: [f32; 4] = [1.0; 4];

/// `run`'s outline, stretched, standing on the bottom of its box `pad`
/// pixels in from the pixmap's edge.
pub(super) fn draw(shaper: &mut Shaper, pixmap: &mut Pixmap, run: &GlyphRun, scale: f32, pad: f32) {
    let Some(shape) = shaper.glyph(run) else {
        return;
    };
    let top = pad;
    let baseline = top + (shape.cap_height * run.stretch * scale).ceil();
    let drop = match run.paint {
        GlyphPaint::Shadow { .. } => SHADOW_DROP * run.size * scale,
        GlyphPaint::Face { .. } | GlyphPaint::Coverage => 0.0,
    };
    let place = Transform::from_row(scale, 0.0, 0.0, -scale * run.stretch, pad, baseline + drop);
    let Some(path) = shape.path.and_then(|path| path.transform(place)) else {
        return;
    };
    match run.paint {
        GlyphPaint::Face { top: upper, bottom } if upper == bottom => fill(pixmap, &path, upper),
        GlyphPaint::Face { top: upper, bottom } => gradient_fill(
            pixmap,
            &path,
            Point::from_xy(0.0, top),
            Point::from_xy(0.0, baseline),
            [upper, bottom],
        ),
        GlyphPaint::Coverage => fill(pixmap, &path, WHITE),
        GlyphPaint::Shadow { alpha } => {
            fill(pixmap, &path, [0.0, 0.0, 0.0, alpha]);
            box_blur(pixmap, (SHADOW_SPREAD * run.size * scale).round() as usize);
        }
    }
}

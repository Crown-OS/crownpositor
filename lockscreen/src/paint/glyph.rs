use tiny_skia::{Pixmap, Point, Transform};

use crate::{
    clock::{Glass, TOP_SHINE_REACH},
    layout::{GlyphPaint, GlyphRun, SHADOW_DROP, SHADOW_SPREAD, Shaper},
    util::blur::box_blur,
};

use super::shape::{fill, gradient_fill};

const WHITE: [f32; 4] = [1.0; 4];
/// Of the shadow's darkness: what is left at the glyph's top, so it gathers
/// at the base.
const SHADOW_TOP: f32 = 0.5;

/// `run`'s outline, stretched, standing on the bottom of its box `pad`
/// pixels in from the pixmap's edge.
pub(super) fn draw(shaper: &mut Shaper, pixmap: &mut Pixmap, run: &GlyphRun, scale: f32, pad: f32) {
    let Some(shape) = shaper.glyph(run) else {
        return;
    };
    let top = pad;
    let height = (shape.cap_height * shape.stretch * scale).ceil();
    let baseline = top + height;
    let drop = match run.paint {
        GlyphPaint::Shadow { .. } => SHADOW_DROP * height,
        _ => 0.0,
    };
    let place = Transform::from_row(
        scale,
        0.0,
        0.0,
        -scale * shape.stretch,
        pad,
        baseline + drop,
    );
    let Some(path) = shape.path.and_then(|path| path.transform(place)) else {
        return;
    };
    match run.paint {
        GlyphPaint::Glass { whiteness } => {
            let glass = Glass::at(whiteness);
            let white = |down| [1.0, 1.0, 1.0, glass.white_at(down, 0.0)];
            let shine_end = top + (baseline - top) * TOP_SHINE_REACH;
            gradient_fill(
                pixmap,
                &path,
                Point::from_xy(0.0, top),
                Point::from_xy(0.0, shine_end),
                &[white(0.0), white(TOP_SHINE_REACH)],
            );
        }
        GlyphPaint::Coverage => fill(pixmap, &path, WHITE),
        GlyphPaint::Shadow { alpha } => {
            let shade = |share: f32| [0.0, 0.0, 0.0, alpha * share];
            gradient_fill(
                pixmap,
                &path,
                Point::from_xy(0.0, top + drop),
                Point::from_xy(0.0, baseline + drop),
                &[shade(SHADOW_TOP), shade(1.0)],
            );
            box_blur(pixmap, (SHADOW_SPREAD * height).round() as usize);
        }
    }
}

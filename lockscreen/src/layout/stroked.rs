//! Clock digits drawn as strokes along a skeleton. A taller clock lengthens
//! their straight runs while bowls stay round and strokes keep their width,
//! where stretching a font's outline would flatten one and fatten the other.

mod frame;
mod grotesk;
mod ribbon;
mod sans;
mod serif;
mod skeleton;

use tiny_skia::{PathBuilder, Point};

use super::{GlyphRun, outline::GlyphShape};
use frame::Frame;
use ribbon::Pen;
use skeleton::Line;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrokedFont {
    /// Monoline and condensed, square-ended, its bowls half circles.
    Sans,
    /// Monoline with oval bowls.
    Grotesk,
    /// High contrast and condensed: heavy stems, hairline curves and bars.
    Serif,
}

/// Of the size: the height digits stand at before they are stretched.
const CAP: f32 = 0.72;
/// Of the size: the stroke at the lightest and the heaviest weight.
const STROKE: [f32; 2] = [0.024, 0.2];
const WEIGHTS: [f32; 2] = [100.0, 900.0];

/// What sets one font's digits apart.
struct Style {
    /// Of the size: how wide the skeleton is.
    width: f32,
    /// Of the size: the room either side of a digit.
    bearing: f32,
    /// Of the size: the room either side of the colon.
    colon_bearing: f32,
    /// How many times the weight's stroke the stems draw.
    stem_gain: f32,
    /// How many times heavier stems are than level strokes.
    contrast: f32,
    /// How many times taller than wide the font's turns are.
    aspect: f32,
    /// Of the stem: a colon dot's radius.
    dot_radius: f32,
    /// Of the glyph's height: how far each colon dot sits from its middle.
    dot_spread: f32,
    digits: &'static [&'static [Line]; 10],
}

impl StrokedFont {
    fn style(self) -> &'static Style {
        match self {
            Self::Sans => &sans::STYLE,
            Self::Grotesk => &grotesk::STYLE,
            Self::Serif => &serif::STYLE,
        }
    }
}

/// `run`'s character drawn in `font` at its stretched height, or `None` for
/// a character the font has no skeleton for.
pub(super) fn glyph(font: StrokedFont, run: &GlyphRun) -> Option<GlyphShape> {
    let style = font.style();
    let stem = stroke_width(run) * style.stem_gain;
    let pen = Pen {
        stem,
        hairline: stem / style.contrast,
    };
    let height = height(run.size, run.stretch);
    if run.ch == ':' {
        return Some(colon(style, pen, height, run.size));
    }
    let lines = style.digits.get(run.ch.to_digit(10)? as usize)?;
    let frame = Frame {
        width: style.width * run.size,
        height,
        left: style.bearing * run.size + stem / 2.0,
        foot: pen.hairline / 2.0,
        top: height - pen.hairline / 2.0,
        aspect: style.aspect,
    };
    let mut path = PathBuilder::new();
    for line in *lines {
        ribbon::push(&mut path, &frame.trace(line), line, pen);
    }
    Some(GlyphShape {
        path: path.finish(),
        advance: frame.width + stem + 2.0 * style.bearing * run.size,
        cap_height: height,
        stretch: 1.0,
    })
}

/// Logical pixels digits `size` logical pixels across stand at, `stretch`
/// times taller than they are drawn.
pub(super) fn height(size: f32, stretch: f32) -> f32 {
    CAP * size * stretch.max(1.0)
}

/// The weight that draws digits `size` across with the stroke `weight` gives
/// them at `stroke_size`, so a clock drawn smaller keeps its stems' width.
/// Past the heaviest weight the stroke keeps growing as it did up to it.
pub(super) fn weight_keeping_stroke(weight: f32, stroke_size: f32, size: f32) -> f32 {
    let stroke = stroke_share(weight) * stroke_size / size.max(f32::EPSILON);
    WEIGHTS[0] + (WEIGHTS[1] - WEIGHTS[0]) * (stroke - STROKE[0]) / (STROKE[1] - STROKE[0])
}

fn stroke_width(run: &GlyphRun) -> f32 {
    run.size * stroke_share(run.weight)
}

/// Of the size: the stroke `weight` draws.
fn stroke_share(weight: f32) -> f32 {
    let share = (weight - WEIGHTS[0]) / (WEIGHTS[1] - WEIGHTS[0]);
    STROKE[0] + (STROKE[1] - STROKE[0]) * share.max(0.0)
}

fn colon(style: &Style, pen: Pen, height: f32, size: f32) -> GlyphShape {
    let radius = pen.stem * style.dot_radius;
    let centre_x = style.colon_bearing * size + radius;
    let mut dots = PathBuilder::new();
    for side in [-1.0, 1.0] {
        let centre = Point::from_xy(centre_x, height * (0.5 + side * style.dot_spread));
        ribbon::push_dot(&mut dots, centre, radius);
    }
    GlyphShape {
        path: dots.finish(),
        advance: 2.0 * (radius + style.colon_bearing * size),
        cap_height: height,
        stretch: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::GlyphPaint;

    const FONTS: [StrokedFont; 3] = [StrokedFont::Sans, StrokedFont::Grotesk, StrokedFont::Serif];

    fn run(ch: char, stretch: f32) -> GlyphRun {
        GlyphRun {
            ch,
            size: 100.0,
            weight: 400.0,
            font: StrokedFont::Sans,
            stretch,
            paint: GlyphPaint::Coverage,
        }
    }

    fn coverage(font: StrokedFont, run: &GlyphRun) -> Option<(tiny_skia::Pixmap, GlyphShape)> {
        let shape = glyph(font, run)?;
        let path = shape.path.clone()?;
        let mut pixmap = tiny_skia::Pixmap::new(
            shape.advance.ceil() as u32 + 8,
            shape.cap_height.ceil() as u32 + 8,
        )?;
        let flip = tiny_skia::Transform::from_row(1.0, 0.0, 0.0, -1.0, 4.0, shape.cap_height + 4.0);
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(tiny_skia::Color::WHITE);
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, flip, None);
        Some((pixmap, shape))
    }

    fn inked(pixmap: &tiny_skia::Pixmap, x: u32, y: u32) -> bool {
        pixmap.pixel(x, y).is_some_and(|pixel| pixel.alpha() > 128)
    }

    #[test]
    fn every_digit_and_the_colon_has_a_shape() {
        for ch in "0123456789:".chars() {
            for font in FONTS {
                let shape = glyph(font, &run(ch, 2.0)).expect("drawn");
                assert!(shape.path.is_some(), "{ch} in {font:?}");
            }
        }
        assert!(glyph(StrokedFont::Sans, &run('A', 1.0)).is_none());
    }

    #[test]
    fn stretching_lengthens_the_digit_but_not_its_stroke() {
        for font in FONTS {
            let bounds = |stretch| {
                let shape = glyph(font, &run('0', stretch)).expect("drawn");
                shape.path.expect("outline").bounds()
            };
            let (short, tall) = (bounds(1.0), bounds(3.0));
            assert!((short.width() - tall.width()).abs() < 0.5, "{font:?}");
            assert!((tall.height() - CAP * 100.0 * 3.0).abs() < 1.0, "{font:?}");
        }
    }

    #[test]
    fn digits_are_as_wide_as_each_other() {
        for font in FONTS {
            let advance = |ch| glyph(font, &run(ch, 1.5)).expect("drawn").advance;
            assert!("123456789".chars().all(|ch| advance(ch) == advance('0')));
        }
    }

    #[test]
    fn heavier_weights_draw_heavier_stems() {
        let stem = |weight| {
            let heavy = GlyphRun {
                weight,
                ..run('1', 2.0)
            };
            let (pixmap, shape) = coverage(StrokedFont::Sans, &heavy).expect("drawn");
            let middle = (shape.cap_height / 2.0) as u32;
            (0..pixmap.width())
                .filter(|x| inked(&pixmap, *x, middle))
                .count()
        };
        assert!(stem(250.0) < stem(500.0) && stem(500.0) < stem(850.0));
    }

    #[test]
    fn the_serif_keeps_its_contrast_when_stretched() {
        let contrast = |stretch| {
            let (pixmap, shape) = coverage(StrokedFont::Serif, &run('0', stretch)).expect("drawn");
            let middle = (shape.cap_height / 2.0 + 4.0) as u32;
            let stems = (0..pixmap.width())
                .filter(|x| inked(&pixmap, *x, middle))
                .count();
            let centre = (shape.advance / 2.0 + 4.0) as u32;
            let bars = (0..pixmap.height())
                .filter(|y| inked(&pixmap, centre, *y))
                .count();
            stems as f32 / bars as f32
        };
        let (natural, stretched) = (contrast(1.0), contrast(3.0));
        assert!(natural > 3.0, "stems only {natural} times the bars");
        assert!(
            (natural - stretched).abs() < 0.5,
            "{natural} against {stretched}"
        );
    }

    #[test]
    fn a_smaller_clock_keeps_its_stroke() {
        let stem = |size, weight| {
            stroke_width(&GlyphRun {
                size,
                weight,
                ..run('1', 1.0)
            })
        };
        let weight = weight_keeping_stroke(700.0, 100.0, 78.0);
        assert!(weight > 700.0);
        assert!((stem(78.0, weight) - stem(100.0, 700.0)).abs() < 1e-3);
    }

    #[test]
    fn colon_dots_spread_with_the_stretch() {
        let gap = |stretch| {
            let shape = glyph(StrokedFont::Sans, &run(':', stretch)).expect("drawn");
            shape.path.expect("dots").bounds().height()
        };
        assert!(gap(3.0) > 2.0 * gap(1.0));
    }
}

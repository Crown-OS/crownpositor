//! Clock digits drawn as strokes along a skeleton. A taller clock lengthens
//! their straight runs while bowls stay round and strokes keep their width,
//! where stretching a font's outline would flatten one and fatten the other.
//! The serif cut draws with an oval pen, its wide axis level, so stems come
//! out heavy and curves hairline, finished with ball terminals.

use tiny_skia::{LineCap, LineJoin, PathBuilder, Stroke, Transform};

use super::{GlyphRun, outline::GlyphShape};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrokedFont {
    Tall,
    Narrow,
    Serif,
}

/// Of the size: the height digits stand at before they are stretched.
const CAP: f32 = 0.72;
/// Of the size: the stroke at the lightest and the heaviest weight.
const STROKE: [f32; 2] = [0.018, 0.13];
const WEIGHTS: [f32; 2] = [100.0, 900.0];
/// Of the size: the room either side of a digit.
const BEARING: f32 = 0.05;
/// Of the skeleton's width: how far each colon dot sits from the middle.
const DOT_SPREAD: f32 = 0.42;
/// Of the stem: a colon dot's radius.
const DOT_RADIUS: f32 = 0.85;
/// Of the stem: a ball terminal's radius.
const BALL_RADIUS: f32 = 0.62;
/// Device pixels per logical pixel the stroke outline stays smooth up to.
const RESOLUTION: f32 = 4.0;

impl StrokedFont {
    /// Of the size: how wide the skeleton is.
    fn width(self) -> f32 {
        match self {
            Self::Tall => 0.40,
            Self::Narrow => 0.33,
            Self::Serif => 0.38,
        }
    }

    /// Of the skeleton's width: the radius its corners turn on.
    fn roundness(self) -> f32 {
        match self {
            Self::Tall | Self::Serif => 0.5,
            Self::Narrow => 0.18,
        }
    }

    fn line_cap(self) -> LineCap {
        match self {
            Self::Tall => LineCap::Round,
            Self::Narrow | Self::Serif => LineCap::Butt,
        }
    }

    /// How many times heavier stems are than level strokes.
    fn contrast(self) -> f32 {
        match self {
            Self::Tall | Self::Narrow => 1.0,
            Self::Serif => 7.0,
        }
    }

    fn balls(self) -> bool {
        self == Self::Serif
    }
}

/// `run`'s character drawn in `font` at its stretched height, or `None` for
/// a character the font has no skeleton for.
pub(super) fn glyph(font: StrokedFont, run: &GlyphRun) -> Option<GlyphShape> {
    let stem = stroke_width(run);
    let frame = Frame {
        width: font.width() * run.size,
        height: CAP * run.size * run.stretch,
        stem,
        hairline: stem / font.contrast(),
        left: BEARING * run.size + stem / 2.0,
    };
    if run.ch == ':' {
        return Some(colon(&frame, run.size));
    }
    let skeleton = skeleton(run.ch)?;
    let mut centre = PathBuilder::new();
    for line in skeleton {
        frame.trace(line, font.roundness() * frame.width, &mut centre);
    }
    let pen = Stroke {
        width: frame.hairline,
        line_cap: font.line_cap(),
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let squeeze = font.contrast();
    let outline = centre
        .finish()?
        .transform(Transform::from_scale(1.0 / squeeze, 1.0))?
        .stroke(&pen, RESOLUTION * squeeze)?
        .transform(Transform::from_scale(squeeze, 1.0))?;
    let mut shape = PathBuilder::new();
    shape.push_path(&outline);
    if font.balls() {
        for [x, y] in skeleton.iter().flat_map(|line| frame.balls(line)) {
            shape.push_circle(x, y, stem * BALL_RADIUS);
        }
    }
    Some(GlyphShape {
        path: shape.finish(),
        advance: frame.width + stem + 2.0 * BEARING * run.size,
        cap_height: frame.height,
        stretch: 1.0,
    })
}

fn stroke_width(run: &GlyphRun) -> f32 {
    let share = (f32::from(run.weight) - WEIGHTS[0]) / (WEIGHTS[1] - WEIGHTS[0]);
    run.size * (STROKE[0] + (STROKE[1] - STROKE[0]) * share.clamp(0.0, 1.0))
}

fn colon(frame: &Frame, size: f32) -> GlyphShape {
    let radius = frame.stem * DOT_RADIUS;
    let centre_x = BEARING * size + radius;
    let mut dots = PathBuilder::new();
    for side in [-1.0, 1.0] {
        let y = frame.y(Y::Mid(side * DOT_SPREAD));
        dots.push_circle(centre_x, y, radius);
    }
    GlyphShape {
        path: dots.finish(),
        advance: 2.0 * (radius + BEARING * size),
        cap_height: frame.height,
        stretch: 1.0,
    }
}

/// A height on the skeleton, so that only the runs between the anchored
/// parts grow with the stretch.
#[derive(Clone, Copy)]
enum Y {
    /// Widths of the skeleton above its foot.
    Foot(f32),
    /// Widths of the skeleton below its top.
    Top(f32),
    /// Widths of the skeleton from its middle, up.
    Mid(f32),
    /// Of the skeleton's height, up from its foot.
    Share(f32),
}

#[derive(Clone, Copy)]
struct Joint {
    /// Of the skeleton's width, from its left.
    x: f32,
    y: Y,
    round: bool,
    /// An end that the serif cut finishes with a ball.
    ball: bool,
}

const fn round(x: f32, y: Y) -> Joint {
    Joint {
        x,
        y,
        round: true,
        ball: false,
    }
}

const fn sharp(x: f32, y: Y) -> Joint {
    Joint {
        x,
        y,
        round: false,
        ball: false,
    }
}

const fn tail(x: f32, y: Y) -> Joint {
    Joint {
        x,
        y,
        round: true,
        ball: true,
    }
}

struct Line {
    closed: bool,
    joints: &'static [Joint],
}

macro_rules! open {
    ($($joint:expr),* $(,)?) => {
        Line { closed: false, joints: &[$($joint),*] }
    };
}

macro_rules! closed {
    ($($joint:expr),* $(,)?) => {
        Line { closed: true, joints: &[$($joint),*] }
    };
}

const FOOT: Y = Y::Foot(0.0);
const TOP: Y = Y::Top(0.0);
const WAIST: f32 = 0.54;
const TAIL: f32 = 0.45;

fn skeleton(ch: char) -> Option<&'static [Line]> {
    DIGITS.get(ch.to_digit(10)? as usize).copied()
}

static DIGITS: [&[Line]; 10] = [
    &[closed!(
        round(0.0, FOOT),
        round(1.0, FOOT),
        round(1.0, TOP),
        round(0.0, TOP),
    )],
    &[open!(
        tail(0.12, Y::Top(0.38)),
        sharp(0.58, TOP),
        round(0.58, FOOT),
    )],
    &[open!(
        tail(0.0, Y::Top(TAIL)),
        round(0.0, TOP),
        round(1.0, TOP),
        round(1.0, Y::Top(0.9)),
        sharp(0.0, FOOT),
        round(1.0, FOOT),
    )],
    &[
        open!(
            tail(0.0, Y::Top(TAIL)),
            round(0.0, TOP),
            round(1.0, TOP),
            round(1.0, Y::Share(WAIST)),
            round(0.3, Y::Share(WAIST)),
        ),
        open!(
            round(0.3, Y::Share(WAIST)),
            round(1.0, Y::Share(WAIST)),
            round(1.0, FOOT),
            round(0.0, FOOT),
            tail(0.0, Y::Foot(TAIL)),
        ),
    ],
    &[
        open!(
            round(0.74, TOP),
            sharp(0.0, Y::Share(0.3)),
            round(1.0, Y::Share(0.3)),
        ),
        open!(round(0.74, Y::Share(0.6)), round(0.74, FOOT)),
    ],
    &[open!(
        round(1.0, TOP),
        sharp(0.02, TOP),
        sharp(0.0, Y::Share(WAIST)),
        round(1.0, Y::Share(WAIST)),
        round(1.0, FOOT),
        round(0.0, FOOT),
        tail(0.0, Y::Foot(TAIL)),
    )],
    &[open!(
        tail(1.0, Y::Top(TAIL)),
        round(1.0, TOP),
        round(0.0, TOP),
        round(0.0, FOOT),
        round(1.0, FOOT),
        round(1.0, Y::Share(WAIST)),
        round(0.0, Y::Share(WAIST)),
        round(0.0, Y::Share(WAIST / 2.0)),
    )],
    &[open!(round(0.0, TOP), sharp(1.0, TOP), round(0.32, FOOT))],
    &[
        closed!(
            round(0.0, FOOT),
            round(1.0, FOOT),
            round(1.0, Y::Share(WAIST)),
            round(0.0, Y::Share(WAIST)),
        ),
        closed!(
            round(0.06, Y::Share(WAIST)),
            round(0.94, Y::Share(WAIST)),
            round(0.94, TOP),
            round(0.06, TOP),
        ),
    ],
    &[open!(
        tail(0.0, Y::Foot(TAIL)),
        round(0.0, FOOT),
        round(1.0, FOOT),
        round(1.0, TOP),
        round(0.0, TOP),
        round(0.0, Y::Share(1.0 - WAIST)),
        round(1.0, Y::Share(1.0 - WAIST)),
        round(1.0, Y::Share(1.0 - WAIST / 2.0)),
    )],
];

/// The box a skeleton is traced in: logical pixels, y up from the baseline.
struct Frame {
    width: f32,
    /// The whole glyph's, stroke included.
    height: f32,
    /// How wide an upright stroke draws.
    stem: f32,
    /// How tall a level stroke draws.
    hairline: f32,
    /// Where the skeleton's left edge sits.
    left: f32,
}

impl Frame {
    fn y(&self, y: Y) -> f32 {
        let (foot, top) = (self.hairline / 2.0, self.height - self.hairline / 2.0);
        match y {
            Y::Foot(widths) => foot + widths * self.width,
            Y::Top(widths) => top - widths * self.width,
            Y::Mid(widths) => self.height / 2.0 + widths * self.width,
            Y::Share(share) => foot + share * (top - foot),
        }
    }

    fn point(&self, joint: &Joint) -> [f32; 2] {
        [self.left + joint.x * self.width, self.y(joint.y)]
    }

    /// Where `line` ends in a ball.
    fn balls<'a>(&'a self, line: &'a Line) -> impl Iterator<Item = [f32; 2]> + 'a {
        line.joints
            .iter()
            .filter(|joint| joint.ball)
            .map(|joint| self.point(joint))
    }

    /// `line`'s centre, its round joints turned on `radius`.
    fn trace(&self, line: &Line, radius: f32, path: &mut PathBuilder) {
        let points: Vec<_> = line.joints.iter().map(|joint| self.point(joint)).collect();
        let count = points.len();
        if count < 2 {
            return;
        }
        let at = |index: usize| points[index % count];
        let corners = if line.closed { 0..count } else { 1..count - 1 };
        let start = if line.closed {
            midpoint(at(count - 1), at(0))
        } else {
            at(0)
        };
        path.move_to(start[0], start[1]);
        for index in corners {
            let previous = at(index + count - 1);
            let next = at(index + 1);
            let reach = |from: usize, to: usize| {
                let ends = !line.closed && (from == 0 || to == count - 1);
                distance(at(from), at(to)) * if ends { 1.0 } else { 0.5 }
            };
            let room = reach(index + count - 1, index).min(reach(index, index + 1));
            let turn_radius = if line.joints[index % count].round {
                radius
            } else {
                0.0
            };
            corner(path, previous, at(index), next, turn_radius, room);
        }
        if line.closed {
            path.close();
        } else {
            let [x, y] = at(count - 1);
            path.line_to(x, y);
        }
    }
}

/// The path through `point` from `previous` towards `next`, its turn rounded
/// on `radius` as far as `room` along either side allows.
fn corner(
    path: &mut PathBuilder,
    previous: [f32; 2],
    point: [f32; 2],
    next: [f32; 2],
    radius: f32,
    room: f32,
) {
    let (Some(incoming), Some(outgoing)) = (direction(previous, point), direction(point, next))
    else {
        path.line_to(point[0], point[1]);
        return;
    };
    let turn = dot(incoming, outgoing).clamp(-1.0, 1.0).acos();
    let half_turn_tan = (turn / 2.0).tan();
    if radius <= 0.0 || turn < 1e-3 || !half_turn_tan.is_finite() || half_turn_tan > 1e3 {
        path.line_to(point[0], point[1]);
        return;
    }
    let along = (radius * half_turn_tan).min(room);
    let radius = along / half_turn_tan;
    let handle = 4.0 / 3.0 * (turn / 4.0).tan() * radius;
    let enter = offset(point, incoming, -along);
    let leave = offset(point, outgoing, along);
    let [first, second] = [
        offset(enter, incoming, handle),
        offset(leave, outgoing, -handle),
    ];
    path.line_to(enter[0], enter[1]);
    path.cubic_to(first[0], first[1], second[0], second[1], leave[0], leave[1]);
}

fn direction(from: [f32; 2], to: [f32; 2]) -> Option<[f32; 2]> {
    let length = distance(from, to);
    (length > f32::EPSILON).then(|| [(to[0] - from[0]) / length, (to[1] - from[1]) / length])
}

fn distance(from: [f32; 2], to: [f32; 2]) -> f32 {
    (to[0] - from[0]).hypot(to[1] - from[1])
}

fn dot(left: [f32; 2], right: [f32; 2]) -> f32 {
    left[0] * right[0] + left[1] * right[1]
}

fn offset(point: [f32; 2], direction: [f32; 2], by: f32) -> [f32; 2] {
    [point[0] + direction[0] * by, point[1] + direction[1] * by]
}

fn midpoint(from: [f32; 2], to: [f32; 2]) -> [f32; 2] {
    [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{FontFamily, GlyphPaint};

    fn run(ch: char, stretch: f32) -> GlyphRun {
        GlyphRun {
            ch,
            size: 100.0,
            weight: 300,
            family: FontFamily::Stroked(StrokedFont::Tall),
            stretch,
            paint: GlyphPaint::Coverage,
        }
    }

    #[test]
    fn every_digit_and_the_colon_has_a_shape() {
        for ch in "0123456789:".chars() {
            for font in [StrokedFont::Tall, StrokedFont::Narrow, StrokedFont::Serif] {
                let shape = glyph(font, &run(ch, 2.0)).expect("drawn");
                assert!(shape.path.is_some(), "{ch} in {font:?}");
            }
        }
        assert!(glyph(StrokedFont::Tall, &run('A', 1.0)).is_none());
    }

    #[test]
    fn stretching_lengthens_the_digit_but_not_its_stroke() {
        let bounds = |stretch| {
            let shape = glyph(StrokedFont::Tall, &run('0', stretch)).expect("drawn");
            shape.path.expect("outline").bounds()
        };
        let (short, tall) = (bounds(1.0), bounds(3.0));
        assert!((short.width() - tall.width()).abs() < 0.5);
        assert!((tall.height() - 3.0 * short.height()).abs() < 1.0);
        assert!((tall.height() - CAP * 100.0 * 3.0).abs() < 1.0);
    }

    #[test]
    fn digits_are_as_wide_as_each_other() {
        let advance = |ch| {
            glyph(StrokedFont::Narrow, &run(ch, 1.5))
                .expect("drawn")
                .advance
        };
        assert!("123456789".chars().all(|ch| advance(ch) == advance('0')));
    }

    #[test]
    fn the_serif_cut_draws_heavy_stems_and_hairline_bars() {
        let shape = glyph(StrokedFont::Serif, &run('0', 1.0)).expect("drawn");
        let mut pixmap = tiny_skia::Pixmap::new(60, 80).expect("size");
        let path = shape.path.expect("outline");
        let flip = Transform::from_row(1.0, 0.0, 0.0, -1.0, 0.0, 76.0);
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(tiny_skia::Color::WHITE);
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, flip, None);
        let inked = |x: u32, y: u32| pixmap.pixel(x, y).is_some_and(|pixel| pixel.alpha() > 128);
        let middle = 76 - (shape.cap_height / 2.0) as u32;
        let stem = (0..60).filter(|x| inked(*x, middle)).count();
        let centre = (shape.advance / 2.0) as u32;
        let bar = (0..80).filter(|y| inked(centre, *y)).count();
        assert!(stem > 3 * bar, "stems {stem}px against bars {bar}px");
    }
}

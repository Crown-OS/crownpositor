/// A height on the skeleton, so that only the runs between the anchored
/// parts grow with the stretch.
#[derive(Debug, Clone, Copy)]
pub(super) enum Y {
    /// Widths of the skeleton above its foot.
    Foot(f32),
    /// Widths of the skeleton below its top.
    Top(f32),
    /// Of the skeleton's height, up from its foot.
    Share(f32),
    /// A share of the skeleton's height, then widths of the skeleton more:
    /// a height that grows with the stretch more slowly than a share.
    Split(f32, f32),
    /// The top of the ink, where a square cut level with it ends a stem.
    Brim,
    /// The baseline.
    Base,
}

/// How an open end of a line is cut off.
#[derive(Debug, Clone, Copy)]
pub(super) enum Cut {
    /// Square to the stroke.
    Square,
    /// Along a line this many degrees counter-clockwise from level.
    Slant(f32),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Joint {
    /// Of the skeleton's width, from its left.
    pub x: f32,
    pub y: Y,
    /// Of the skeleton's width: the radius the line turns on here, none for
    /// a sharp corner.
    pub radius: f32,
    /// How much of the pen's stem the stroke keeps here: 1 for the full
    /// stem, 0 for a hairline all round.
    pub heft: f32,
}

/// Of the skeleton's width: the radius of a bowl's turn, so that a bowl as
/// wide as the skeleton ends in a half circle.
pub(super) const BOWL: f32 = 0.5;

pub(super) const fn bowl(x: f32, y: Y) -> Joint {
    bend(x, y, BOWL)
}

pub(super) const fn bend(x: f32, y: Y, radius: f32) -> Joint {
    Joint {
        x,
        y,
        radius,
        heft: 1.0,
    }
}

pub(super) const fn sharp(x: f32, y: Y) -> Joint {
    bend(x, y, 0.0)
}

impl Joint {
    pub(super) const fn heft(self, heft: f32) -> Self {
        Self { heft, ..self }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Line {
    pub closed: bool,
    pub joints: &'static [Joint],
    pub start: Cut,
    pub end: Cut,
}

impl Line {
    pub(super) const fn open(joints: &'static [Joint]) -> Self {
        Self {
            closed: false,
            joints,
            start: Cut::Square,
            end: Cut::Square,
        }
    }

    pub(super) const fn closed(joints: &'static [Joint]) -> Self {
        Self {
            closed: true,
            ..Self::open(joints)
        }
    }

    pub(super) const fn cut(self, start: Cut, end: Cut) -> Self {
        Self { start, end, ..self }
    }
}

pub(super) const FOOT: Y = Y::Foot(0.0);
pub(super) const TOP: Y = Y::Top(0.0);
pub(super) const LEVEL: Cut = Cut::Slant(0.0);
pub(super) const SQUARE: Cut = Cut::Square;

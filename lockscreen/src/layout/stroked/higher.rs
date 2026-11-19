use super::Style;
use super::ribbon::Dot;
use super::skeleton::{Cut, FLUSH, FOOT, LEVEL, Line, SQUARE, TOP, Y, sharp};

pub(super) static STYLE: Style = Style {
    width: 0.365,
    bearing: 0.06,
    colon_bearing: 0.06,
    stem_gain: 1.0,
    contrast: 1.0,
    aspect: 1.0,
    dot: Dot::Square,
    dot_radius: 0.5,
    dot_spread: 0.218,
    digits: &DIGITS,
};

/// Where the low bars of 2 to 9 run, close over the foot.
const BAR: Y = Y::Foot(0.76);
/// Where 0's slanted bar meets its left and its right side.
const ZERO_BAR: [Y; 2] = [Y::Foot(0.76), Y::Foot(0.9)];
/// Of the skeleton's width: where 1's flag starts, and the degrees its
/// chamfer rises at.
const ONE_FLAG: f32 = -0.1;
const ONE_CHAMFER: f32 = 34.0;

static DIGITS: [&[Line]; 10] = [
    &[
        Line::closed(&[
            sharp(0.0, FOOT),
            sharp(1.0, FOOT),
            sharp(1.0, TOP),
            sharp(0.0, TOP),
        ]),
        Line::open(&[sharp(0.0, ZERO_BAR[0]), sharp(1.0, ZERO_BAR[1])]),
    ],
    &[
        Line::open(&[sharp(ONE_FLAG, TOP), sharp(1.0, TOP), sharp(1.0, Y::Base)])
            .cut(Cut::Slant(ONE_CHAMFER), LEVEL),
    ],
    &[Line::open(&[
        sharp(0.0, TOP),
        sharp(1.0, TOP),
        sharp(1.0, BAR),
        sharp(0.0, BAR),
        sharp(0.0, FOOT),
        sharp(1.0, FOOT),
    ])
    .cut(FLUSH, FLUSH)],
    &[
        Line::open(&[
            sharp(0.0, TOP),
            sharp(1.0, TOP),
            sharp(1.0, FOOT),
            sharp(0.0, FOOT),
        ])
        .cut(FLUSH, FLUSH),
        Line::open(&[sharp(0.0, BAR), sharp(1.0, BAR)]).cut(FLUSH, SQUARE),
    ],
    &[
        Line::open(&[sharp(0.0, Y::Brim), sharp(0.0, BAR), sharp(1.0, BAR)]).cut(LEVEL, SQUARE),
        Line::open(&[sharp(1.0, Y::Brim), sharp(1.0, Y::Base)]).cut(LEVEL, LEVEL),
    ],
    &[Line::open(&[
        sharp(1.0, TOP),
        sharp(0.0, TOP),
        sharp(0.0, BAR),
        sharp(1.0, BAR),
        sharp(1.0, FOOT),
        sharp(0.0, FOOT),
    ])
    .cut(FLUSH, FLUSH)],
    &[Line::open(&[
        sharp(1.0, TOP),
        sharp(0.0, TOP),
        sharp(0.0, FOOT),
        sharp(1.0, FOOT),
        sharp(1.0, BAR),
        sharp(0.0, BAR),
    ])
    .cut(FLUSH, SQUARE)],
    &[Line::open(&[sharp(0.0, TOP), sharp(1.0, TOP), sharp(1.0, Y::Base)]).cut(FLUSH, LEVEL)],
    &[
        Line::closed(&[
            sharp(0.0, FOOT),
            sharp(1.0, FOOT),
            sharp(1.0, TOP),
            sharp(0.0, TOP),
        ]),
        Line::open(&[sharp(0.0, BAR), sharp(1.0, BAR)]),
    ],
    &[Line::open(&[
        sharp(1.0, BAR),
        sharp(0.0, BAR),
        sharp(0.0, TOP),
        sharp(1.0, TOP),
        sharp(1.0, Y::Base),
    ])
    .cut(SQUARE, LEVEL)],
];

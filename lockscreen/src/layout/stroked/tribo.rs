use super::Style;
use super::ribbon::Dot;
use super::skeleton::{Cut, FLUSH, FOOT, LEVEL, Line, SQUARE, TOP, Y, sharp};

pub(super) static STYLE: Style = Style {
    width: 0.24,
    bearing: 0.04,
    colon_bearing: 0.05,
    stem_gain: 0.5,
    contrast: 1.0,
    aspect: 1.0,
    dot: Dot::Square,
    dot_radius: 0.5,
    dot_spread: 0.2,
    digits: &DIGITS,
};

/// Of the skeleton's width: how far a corner's chamfer cuts in, the small
/// one of its O and the large one of its C and G.
const SMALL: f32 = 0.22;
const LARGE: f32 = 0.42;
/// Of the skeleton's height: where the bars between two counters run.
const WAIST: f32 = 0.5;
const WAIST_BAR: Y = Y::Share(WAIST);
/// Of the skeleton's width: how far the spur past a corner reaches, cut
/// back at 45 degrees.
const SPUR: f32 = 0.33;
const SPUR_CUT: Cut = Cut::Slant(45.0);
/// How far the ears of 2, 6 and 9 reach from their bar.
const EAR: f32 = 0.45;
/// Where 2's diagonal leaves its right side and lands on its left.
const TWO_DIAGONAL: [Y; 2] = [Y::Share(0.62), Y::Share(0.38)];
/// Of the skeleton's width: where 3's middle bar starts.
const THREE_BAR: f32 = 0.25;
/// Where 4's bar runs, and how far past its stem it reaches.
const FOUR_BAR: Y = Y::Share(0.38);
const FOUR_BAR_END: f32 = 1.2;
/// Of the skeleton's width: where 1's stem stands and its flag starts.
const ONE_STEM: f32 = 0.6;
const ONE_FLAG: f32 = 0.1;
/// Where 7's right side turns down-left, and where its foot stands.
const SEVEN_TURN: f32 = 0.6;
const SEVEN_FOOT: f32 = 0.35;

static DIGITS: [&[Line]; 10] = [
    &[
        Line::closed(&[
            sharp(0.0, FOOT),
            sharp(1.0, FOOT),
            sharp(1.0, TOP),
            sharp(SMALL, TOP),
            sharp(0.0, Y::Top(SMALL)),
        ]),
        Line::open(&[sharp(1.0, TOP), sharp(1.0 + SPUR, TOP)]).cut(SQUARE, SPUR_CUT),
    ],
    &[Line::open(&[
        sharp(ONE_FLAG, Y::Top(ONE_STEM - ONE_FLAG)),
        sharp(ONE_STEM, TOP),
        sharp(ONE_STEM, Y::Base),
    ])
    .cut(LEVEL, LEVEL)],
    &[Line::open(&[
        sharp(0.0, Y::Top(EAR)),
        sharp(0.0, Y::Top(SMALL)),
        sharp(SMALL, TOP),
        sharp(1.0, TOP),
        sharp(1.0, TWO_DIAGONAL[0]),
        sharp(0.0, TWO_DIAGONAL[1]),
        sharp(0.0, FOOT),
        sharp(1.0, FOOT),
    ])
    .cut(LEVEL, FLUSH)],
    &[
        Line::open(&[
            sharp(0.0, TOP),
            sharp(1.0 - LARGE, TOP),
            sharp(1.0, Y::Top(LARGE)),
            sharp(1.0, Y::Foot(LARGE)),
            sharp(1.0 - LARGE, FOOT),
            sharp(0.0, FOOT),
        ])
        .cut(FLUSH, FLUSH),
        Line::open(&[sharp(THREE_BAR, WAIST_BAR), sharp(1.0, WAIST_BAR)]),
    ],
    &[
        Line::open(&[
            sharp(0.0, Y::Brim),
            sharp(0.0, FOUR_BAR),
            sharp(FOUR_BAR_END, FOUR_BAR),
        ])
        .cut(SPUR_CUT, SPUR_CUT),
        Line::open(&[sharp(1.0, Y::Brim), sharp(1.0, Y::Base)]).cut(SPUR_CUT, LEVEL),
    ],
    &[Line::open(&[
        sharp(1.0, TOP),
        sharp(0.0, TOP),
        sharp(0.0, WAIST_BAR),
        sharp(1.0 - LARGE, WAIST_BAR),
        sharp(1.0, Y::Split(WAIST, -LARGE)),
        sharp(1.0, FOOT),
        sharp(0.0, FOOT),
    ])
    .cut(FLUSH, FLUSH)],
    &[Line::open(&[
        sharp(1.0, Y::Top(EAR)),
        sharp(1.0, TOP),
        sharp(LARGE, TOP),
        sharp(0.0, Y::Top(LARGE)),
        sharp(0.0, FOOT),
        sharp(1.0, FOOT),
        sharp(1.0, WAIST_BAR),
        sharp(0.0, WAIST_BAR),
    ])
    .cut(LEVEL, SQUARE)],
    &[Line::open(&[
        sharp(0.0, TOP),
        sharp(1.0, TOP),
        sharp(1.0, Y::Share(SEVEN_TURN)),
        sharp(SEVEN_FOOT, Y::Split(SEVEN_TURN, SEVEN_FOOT - 1.0)),
        sharp(SEVEN_FOOT, Y::Base),
    ])
    .cut(FLUSH, LEVEL)],
    &[
        Line::closed(&[
            sharp(0.0, Y::Foot(SMALL)),
            sharp(SMALL, FOOT),
            sharp(1.0, FOOT),
            sharp(1.0, TOP),
            sharp(SMALL, TOP),
            sharp(0.0, Y::Top(SMALL)),
        ]),
        Line::open(&[sharp(0.0, WAIST_BAR), sharp(1.0, WAIST_BAR)]),
    ],
    &[Line::open(&[
        sharp(0.0, Y::Foot(EAR)),
        sharp(0.0, FOOT),
        sharp(1.0 - LARGE, FOOT),
        sharp(1.0, Y::Foot(LARGE)),
        sharp(1.0, TOP),
        sharp(0.0, TOP),
        sharp(0.0, WAIST_BAR),
        sharp(1.0, WAIST_BAR),
    ])
    .cut(LEVEL, SQUARE)],
];

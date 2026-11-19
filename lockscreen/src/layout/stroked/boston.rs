use super::Style;
use super::ribbon::Dot;
use super::skeleton::{FLUSH, FOOT, LEVEL, Line, SQUARE, TOP, Y, sharp};

pub(super) static STYLE: Style = Style {
    width: 0.24,
    bearing: 0.035,
    colon_bearing: 0.035,
    stem_gain: 0.75,
    contrast: 1.0,
    aspect: 1.0,
    dot: Dot::Square,
    dot_radius: 0.42,
    dot_spread: 0.22,
    digits: &DIGITS,
};

/// Of the skeleton's width: how far a chamfer cuts into a corner.
const C: f32 = 0.35;
/// Of the skeleton's height: where the slanted bars of 2, 3, 7 and 8 cross
/// the middle of the skeleton.
const MIDDLE: f32 = 0.5;
/// Widths of the skeleton a slanted bar rises across the whole skeleton.
const SLANT: f32 = 0.58;
/// Of the skeleton's height: the waists of 5 and 6, and of 9 and 4.
const HIGH_WAIST: f32 = 0.6;
const LOW_WAIST: f32 = 0.4;
/// How far the ears of 2, 3, 6 and 7 reach down from the top.
const TOP_EAR: Y = Y::Top(0.5);
/// How far the ears of 2, 3, 5 and 9 reach up from the foot.
const FOOT_EAR: Y = Y::Foot(0.5);
const FIVE_EAR: Y = Y::Foot(0.75);
/// Of the skeleton's width: where 3's slanted bar starts.
const THREE_BAR: f32 = 0.25;
/// Of the skeleton's width: how far past its stem 4's bar reaches.
const FOUR_BAR_END: f32 = 1.4;
/// Of the skeleton's width: where 1's stem stands and its flag starts.
const ONE_STEM: f32 = 0.6;
const ONE_FLAG: f32 = 0.1;

/// The height a bar slanted up to the right stands at `x` widths across.
const fn slanted(x: f32) -> Y {
    Y::Split(MIDDLE, SLANT * (x - 0.5))
}

static DIGITS: [&[Line]; 10] = [
    &[Line::closed(&[
        sharp(1.0, FOOT),
        sharp(C, FOOT),
        sharp(0.0, Y::Foot(C)),
        sharp(0.0, Y::Top(C)),
        sharp(C, TOP),
        sharp(1.0 - C, TOP),
        sharp(1.0, Y::Top(C)),
    ])],
    &[Line::open(&[
        sharp(ONE_FLAG, TOP),
        sharp(ONE_STEM - C, TOP),
        sharp(ONE_STEM, Y::Top(C)),
        sharp(ONE_STEM, Y::Base),
    ])
    .cut(SQUARE, LEVEL)],
    &[Line::open(&[
        sharp(0.0, TOP_EAR),
        sharp(0.0, TOP),
        sharp(1.0 - C, TOP),
        sharp(1.0, Y::Top(C)),
        sharp(1.0, slanted(1.0)),
        sharp(0.0, slanted(0.0)),
        sharp(0.0, FOOT),
        sharp(1.0, FOOT),
        sharp(1.0, FOOT_EAR),
    ])
    .cut(LEVEL, LEVEL)],
    &[
        Line::open(&[
            sharp(0.0, TOP_EAR),
            sharp(0.0, TOP),
            sharp(1.0 - C, TOP),
            sharp(1.0, Y::Top(C)),
            sharp(1.0, Y::Foot(C)),
            sharp(1.0 - C, FOOT),
            sharp(0.0, FOOT),
            sharp(0.0, FOOT_EAR),
        ])
        .cut(LEVEL, LEVEL),
        Line::open(&[
            sharp(THREE_BAR, slanted(THREE_BAR)),
            sharp(1.0, slanted(1.0)),
        ]),
    ],
    &[
        Line::open(&[
            sharp(0.0, Y::Brim),
            sharp(0.0, Y::Split(LOW_WAIST, C)),
            sharp(C, Y::Share(LOW_WAIST)),
            sharp(FOUR_BAR_END, Y::Share(LOW_WAIST)),
        ])
        .cut(LEVEL, SQUARE),
        Line::open(&[sharp(1.0, Y::Brim), sharp(1.0, Y::Base)]).cut(LEVEL, LEVEL),
    ],
    &[Line::open(&[
        sharp(1.0, TOP),
        sharp(C, TOP),
        sharp(0.0, Y::Top(C)),
        sharp(0.0, Y::Share(HIGH_WAIST)),
        sharp(1.0 - C, Y::Share(HIGH_WAIST)),
        sharp(1.0, Y::Split(HIGH_WAIST, -C)),
        sharp(1.0, FOOT),
        sharp(C, FOOT),
        sharp(0.0, Y::Foot(C)),
        sharp(0.0, FIVE_EAR),
    ])
    .cut(FLUSH, LEVEL)],
    &[Line::open(&[
        sharp(1.0, TOP_EAR),
        sharp(1.0, Y::Top(C)),
        sharp(1.0 - C, TOP),
        sharp(0.0, TOP),
        sharp(0.0, Y::Foot(C)),
        sharp(C, FOOT),
        sharp(1.0, FOOT),
        sharp(1.0, Y::Split(HIGH_WAIST, -C)),
        sharp(1.0 - C, Y::Share(HIGH_WAIST)),
        sharp(0.0, Y::Share(HIGH_WAIST)),
    ])
    .cut(LEVEL, SQUARE)],
    &[Line::open(&[
        sharp(0.0, TOP_EAR),
        sharp(0.0, TOP),
        sharp(1.0, TOP),
        sharp(1.0, slanted(1.0)),
        sharp(0.0, slanted(0.0)),
        sharp(0.0, Y::Base),
    ])
    .cut(LEVEL, LEVEL)],
    &[
        Line::closed(&[
            sharp(0.0, TOP),
            sharp(1.0 - C, TOP),
            sharp(1.0, Y::Top(C)),
            sharp(1.0, FOOT),
            sharp(C, FOOT),
            sharp(0.0, Y::Foot(C)),
        ]),
        Line::open(&[sharp(0.0, slanted(1.0)), sharp(1.0, slanted(0.0))]),
    ],
    &[Line::open(&[
        sharp(0.0, FOOT_EAR),
        sharp(0.0, Y::Foot(C)),
        sharp(C, FOOT),
        sharp(1.0, FOOT),
        sharp(1.0, Y::Top(C)),
        sharp(1.0 - C, TOP),
        sharp(0.0, TOP),
        sharp(0.0, Y::Split(LOW_WAIST, C)),
        sharp(C, Y::Share(LOW_WAIST)),
        sharp(1.0, Y::Share(LOW_WAIST)),
    ])
    .cut(LEVEL, SQUARE)],
];

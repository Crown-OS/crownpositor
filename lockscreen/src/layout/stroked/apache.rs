use super::Style;
use super::ribbon::Dot;
use super::skeleton::{Cut, FLUSH, FOOT, LEVEL, Line, SQUARE, TOP, Y, sharp};

pub(super) static STYLE: Style = Style {
    width: 0.27,
    bearing: 0.04,
    colon_bearing: 0.03,
    stem_gain: 1.0,
    contrast: 1.1,
    aspect: 1.0,
    dot: Dot::Square,
    dot_radius: 0.5,
    dot_spread: 0.3,
    digits: &DIGITS,
};

/// Of the skeleton's width: how far a chamfer cuts into a corner.
const C: f32 = 0.22;
/// Of the skeleton's width: where the narrower digits' right stem stands.
const RIGHT: f32 = 0.92;
/// Of the skeleton's height: where the bars between two counters run.
const WAIST: f32 = 0.505;
const WAIST_BAR: Y = Y::Share(WAIST);
/// How far the ears hanging from the top of 2, 3 and 7 reach down.
const TOP_EAR: Y = Y::Top(0.53);
/// How far the ears rising from the foot of 3, 5 and 9 reach up.
const FOOT_EAR: Y = Y::Foot(0.57);
/// Where 2's diagonal leaves its right stem and lands on its left.
const TWO_DIAGONAL: [Y; 2] = [Y::Foot(1.37), Y::Foot(0.55)];
/// Of the skeleton's width: where 3's tongue starts.
const THREE_TONGUE: f32 = 0.49;
/// Where 4's bar runs, how far past its stem it reaches, and where its
/// diagonal leaves the bar and meets the top.
const FOUR_BAR: Y = Y::Foot(0.49);
const FOUR_BAR_END: f32 = 1.3;
const FOUR_DIAGONAL: [f32; 2] = [0.0, 0.62];
/// Degrees 4's bar is cut at under its diagonal.
const FOUR_CUT: f32 = -58.0;
const FOUR_STEM: f32 = 0.88;
/// Of the skeleton's width: where 1's stem stands, and where its arm
/// starts and meets the top, past the stem so that the arm's edge runs
/// into the stem's corner.
const ONE_STEM: f32 = 0.42;
const ONE_ARM: [f32; 2] = [-0.05, ONE_STEM + 0.1];
/// How far 1's arm reaches down.
const ONE_ARM_FOOT: Y = Y::Top(0.45);
/// Of the skeleton's width: where 7's diagonal lands.
const SEVEN_FOOT: f32 = 0.17;

static DIGITS: [&[Line]; 10] = [
    &[Line::closed(&[
        sharp(0.0, FOOT),
        sharp(1.0 - C, FOOT),
        sharp(1.0, Y::Foot(C)),
        sharp(1.0, TOP),
        sharp(C, TOP),
        sharp(0.0, Y::Top(C)),
    ])],
    &[
        Line::open(&[sharp(ONE_STEM, Y::Brim), sharp(ONE_STEM, Y::Base)]).cut(LEVEL, LEVEL),
        Line::open(&[sharp(ONE_ARM[0], ONE_ARM_FOOT), sharp(ONE_ARM[1], Y::Brim)])
            .cut(LEVEL, LEVEL),
    ],
    &[Line::open(&[
        sharp(0.0, TOP_EAR),
        sharp(0.0, Y::Top(C)),
        sharp(C, TOP),
        sharp(RIGHT, TOP),
        sharp(RIGHT, TWO_DIAGONAL[0]),
        sharp(0.0, TWO_DIAGONAL[1]),
        sharp(0.0, FOOT),
        sharp(RIGHT, FOOT),
    ])
    .cut(LEVEL, FLUSH)],
    &[
        Line::open(&[
            sharp(0.0, TOP_EAR),
            sharp(0.0, Y::Top(C)),
            sharp(C, TOP),
            sharp(1.0, TOP),
            sharp(1.0, Y::Foot(C)),
            sharp(1.0 - C, FOOT),
            sharp(0.0, FOOT),
            sharp(0.0, FOOT_EAR),
        ])
        .cut(LEVEL, LEVEL),
        Line::open(&[sharp(THREE_TONGUE, WAIST_BAR), sharp(1.0, WAIST_BAR)]),
    ],
    &[
        Line::open(&[sharp(0.0, FOUR_BAR), sharp(FOUR_BAR_END, FOUR_BAR)])
            .cut(Cut::Slant(FOUR_CUT), SQUARE),
        Line::open(&[
            sharp(FOUR_DIAGONAL[0], FOUR_BAR),
            sharp(FOUR_DIAGONAL[1], TOP),
            sharp(FOUR_STEM - C, TOP),
            sharp(FOUR_STEM, Y::Top(C)),
            sharp(FOUR_STEM, Y::Base),
        ])
        .cut(LEVEL, LEVEL),
    ],
    &[Line::open(&[
        sharp(1.1, TOP),
        sharp(0.0, TOP),
        sharp(0.0, WAIST_BAR),
        sharp(RIGHT - C, WAIST_BAR),
        sharp(RIGHT, Y::Split(WAIST, -C)),
        sharp(RIGHT, FOOT),
        sharp(C, FOOT),
        sharp(0.0, Y::Foot(C)),
        sharp(0.0, FOOT_EAR),
    ])
    .cut(SQUARE, LEVEL)],
    &[Line::open(&[
        sharp(RIGHT, TOP_EAR),
        sharp(RIGHT, TOP),
        sharp(C, TOP),
        sharp(0.0, Y::Top(C)),
        sharp(0.0, FOOT),
        sharp(RIGHT - C, FOOT),
        sharp(RIGHT, Y::Foot(C)),
        sharp(RIGHT, WAIST_BAR),
        sharp(0.0, WAIST_BAR),
    ])
    .cut(LEVEL, SQUARE)],
    &[Line::open(&[
        sharp(0.0, TOP_EAR),
        sharp(0.0, TOP),
        sharp(1.0 - C, TOP),
        sharp(1.0, Y::Top(C)),
        sharp(SEVEN_FOOT, Y::Base),
    ])
    .cut(LEVEL, LEVEL)],
    &[
        Line::closed(&[
            sharp(0.0, FOOT),
            sharp(1.0 - C, FOOT),
            sharp(1.0, Y::Foot(C)),
            sharp(1.0, TOP),
            sharp(C, TOP),
            sharp(0.0, Y::Top(C)),
        ]),
        Line::open(&[sharp(0.0, WAIST_BAR), sharp(1.0, WAIST_BAR)]),
    ],
    &[Line::open(&[
        sharp(0.0, FOOT_EAR),
        sharp(0.0, FOOT),
        sharp(RIGHT - C, FOOT),
        sharp(RIGHT, Y::Foot(C)),
        sharp(RIGHT, TOP),
        sharp(C, TOP),
        sharp(0.0, Y::Top(C)),
        sharp(0.0, WAIST_BAR),
        sharp(RIGHT, WAIST_BAR),
    ])
    .cut(LEVEL, SQUARE)],
];

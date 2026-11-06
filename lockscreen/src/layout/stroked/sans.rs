use super::Style;
use super::skeleton::{Cut, FOOT, LEVEL, Line, SQUARE, TOP, Y, bend, bowl, sharp};

pub(super) static STYLE: Style = Style {
    width: 0.335,
    bearing: 0.05,
    colon_bearing: 0.07,
    stem_gain: 1.0,
    contrast: 1.0,
    aspect: 1.0,
    dot_radius: 0.45,
    dot_spread: 0.218,
    digits: &DIGITS,
};

/// Where the lower bowls of 3 and 5 top out, and 6's bowl.
const WAIST: Y = Y::Split(0.522, 0.13);
const SIX_WAIST: Y = Y::Split(0.519, 0.13);
/// Where the upper bowl of 9 bottoms out.
const NINE_WAIST: Y = Y::Split(0.514, -0.13);
/// Where 3's diagonal meets its bowl.
const THREE_JUNCTION: Y = Y::Split(0.495, 0.13);
/// Of the skeleton's height: where 8's bowls meet.
const EIGHT_WAIST: f32 = 0.514;
/// Where the terminals hanging from a top bowl end: 2's and 6's.
const TWO_TERMINAL: Y = Y::Split(0.636, 0.24);
const SIX_TERMINAL: Y = Y::Split(0.719, 0.24);
/// Where the terminals rising from a foot bowl end: 3's, 5's and 9's.
const FOOT_TERMINAL: Y = Y::Split(0.334, -0.24);
const NINE_TERMINAL: Y = Y::Split(0.379, -0.24);
/// Degrees a terminal's cut falls toward its counter.
const TERMINAL_SLANT: f32 = 12.0;
/// Degrees a foot terminal's cut rises toward its counter.
const FOOT_SLANT: f32 = 15.0;
/// Where 2's right side turns into its diagonal, and the turn's radius in
/// widths of the skeleton: wide enough to bend the whole diagonal.
const TWO_BEND: Y = Y::Share(0.48);
const TWO_BEND_RADIUS: f32 = 7.0;
/// Of the skeleton's width: where 2's foot ends, flush with its stem.
const FOOT_BAR_END: f32 = 1.24;
/// Of the skeleton's width: where the top bars of 3, 5 and 7 start.
const TOP_BAR_START: f32 = -0.2;
/// Of the skeleton's height: where 4's bar runs and its stem starts.
const FOUR_BAR: f32 = 0.187;
const FOUR_STEM_TOP: f32 = 0.67;
/// Of the skeleton's width: where 4's stem stands and its bar ends.
const FOUR_STEM: f32 = 0.83;
const FOUR_BAR_END: f32 = 1.3;
/// Of the skeleton's width: where 1's stem stands.
const ONE_STEM: f32 = 0.64;
/// Where 7's stroke bends on its way down, and the bend's radius.
const SEVEN_BEND: Y = Y::Share(0.626);
const SEVEN_BEND_RADIUS: f32 = 34.0;

pub(super) static DIGITS: [&[Line]; 10] = [
    &[Line::closed(&[
        bowl(0.0, FOOT),
        bowl(1.0, FOOT),
        bowl(1.0, TOP),
        bowl(0.0, TOP),
    ])],
    &[
        Line::open(&[sharp(ONE_STEM, Y::Brim), sharp(ONE_STEM, Y::Base)]).cut(LEVEL, LEVEL),
        Line::open(&[sharp(-0.24, Y::Top(0.62)), sharp(0.55, Y::Top(-0.05))])
            .cut(Cut::Slant(-65.0), SQUARE),
    ],
    &[Line::open(&[
        sharp(0.0, TWO_TERMINAL),
        bowl(0.0, TOP),
        bowl(1.0, TOP),
        bend(1.0, TWO_BEND, TWO_BEND_RADIUS),
        sharp(0.0, FOOT),
        sharp(FOOT_BAR_END, FOOT),
    ])
    .cut(Cut::Slant(-TERMINAL_SLANT), SQUARE)],
    &[
        Line::open(&[
            sharp(TOP_BAR_START, TOP),
            sharp(1.0, TOP),
            sharp(0.05, THREE_JUNCTION),
        ])
        .cut(SQUARE, Cut::Slant(-29.0)),
        Line::open(&[
            sharp(0.2, WAIST),
            bowl(1.0, WAIST),
            bowl(1.0, FOOT),
            bowl(0.0, FOOT),
            sharp(0.0, FOOT_TERMINAL),
        ])
        .cut(SQUARE, Cut::Slant(FOOT_SLANT)),
    ],
    &[
        Line::open(&[sharp(0.42, Y::Brim), sharp(-0.22, Y::Share(FOUR_BAR))]).cut(LEVEL, LEVEL),
        Line::open(&[
            sharp(-0.44, Y::Share(FOUR_BAR)),
            sharp(FOUR_BAR_END, Y::Share(FOUR_BAR)),
        ]),
        Line::open(&[
            sharp(FOUR_STEM, Y::Share(FOUR_STEM_TOP)),
            sharp(FOUR_STEM, Y::Base),
        ])
        .cut(Cut::Slant(20.0), LEVEL),
    ],
    &[Line::open(&[
        sharp(1.0, TOP),
        sharp(0.0, TOP),
        sharp(0.0, WAIST),
        bowl(1.0, WAIST),
        bowl(1.0, FOOT),
        bowl(0.0, FOOT),
        sharp(0.0, FOOT_TERMINAL),
    ])
    .cut(SQUARE, Cut::Slant(FOOT_SLANT))],
    &[Line::open(&[
        sharp(1.0, SIX_TERMINAL),
        bowl(1.0, TOP),
        bowl(0.0, TOP),
        bowl(0.0, FOOT),
        bowl(1.0, FOOT),
        bowl(1.0, SIX_WAIST),
        bowl(0.0, SIX_WAIST),
        sharp(0.0, Y::Share(0.3)),
    ])
    .cut(Cut::Slant(TERMINAL_SLANT), SQUARE)],
    &[Line::open(&[
        sharp(TOP_BAR_START, TOP),
        sharp(1.06, TOP),
        bend(0.5, SEVEN_BEND, SEVEN_BEND_RADIUS),
        sharp(0.22, Y::Base),
    ])
    .cut(SQUARE, LEVEL)],
    &[
        Line::closed(&[
            bowl(0.0, FOOT),
            bowl(1.0, FOOT),
            bowl(1.0, Y::Share(EIGHT_WAIST)),
            bowl(0.0, Y::Share(EIGHT_WAIST)),
        ]),
        Line::closed(&[
            bowl(0.0, Y::Share(EIGHT_WAIST)),
            bowl(0.97, Y::Share(EIGHT_WAIST)),
            bowl(0.97, TOP),
            bowl(0.0, TOP),
        ]),
    ],
    &[Line::open(&[
        sharp(1.0, Y::Share(0.7)),
        bowl(1.0, NINE_WAIST),
        bowl(0.0, NINE_WAIST),
        bowl(0.0, TOP),
        bowl(1.0, TOP),
        bowl(1.0, FOOT),
        bowl(0.0, FOOT),
        sharp(0.0, NINE_TERMINAL),
    ])
    .cut(SQUARE, Cut::Slant(TERMINAL_SLANT))],
];

use super::Style;
use super::skeleton::{Cut, FOOT, LEVEL, Line, SQUARE, TOP, Y, bowl, sharp};

pub(super) static STYLE: Style = Style {
    width: 0.30,
    bearing: 0.06,
    colon_bearing: 0.07,
    stem_gain: 1.15,
    contrast: 5.0,
    aspect: 1.8,
    dot_radius: 0.39,
    dot_spread: 0.226,
    digits: &DIGITS,
};

/// Widths of the skeleton a bowl's turn rises through: its radius drawn
/// `STYLE.aspect` times taller.
const TURN: f32 = 0.9;
/// How much of the stem a bowl keeps at the middle of its turns, so that
/// it thins toward its hairline.
const TURN_HEFT: f32 = 0.9;
/// Of the skeleton's height: where the upper bowl of 9 bottoms out.
const NINE_WAIST: Y = Y::Share(0.435);
/// Of the skeleton's height: where 4's bar runs.
const BAR: f32 = 0.33;
/// Of the skeleton's width: how far a foot serif reaches either side of its stem.
const SERIF_REACH: f32 = 0.7;
/// Of the stem: how much wider a stem flares where it meets its foot serif.
const FLARE: f32 = 1.6;
/// Widths of the skeleton above the foot where a stem starts to flare.
const FLARE_RISE: f32 = 0.4;
const ONE_STEM: f32 = 0.43;
const FOUR_STEM: f32 = 0.66;

const fn turn(x: f32, y: Y) -> super::skeleton::Joint {
    bowl(x, y).heft(TURN_HEFT)
}

static DIGITS: [&[Line]; 10] = [
    &[Line::closed(&[
        sharp(0.0, Y::Foot(TURN)),
        turn(0.0, FOOT),
        turn(1.0, FOOT),
        sharp(1.0, Y::Foot(TURN)),
        sharp(1.0, Y::Top(TURN)),
        turn(1.0, TOP),
        turn(0.0, TOP),
        sharp(0.0, Y::Top(TURN)),
    ])],
    &[
        Line::open(&[
            sharp(ONE_STEM, Y::Brim),
            sharp(ONE_STEM, Y::Foot(FLARE_RISE)),
            sharp(ONE_STEM, Y::Base).heft(FLARE),
        ])
        .cut(Cut::Slant(12.0), LEVEL),
        Line::open(&[
            sharp(0.3, Y::Top(0.05)).heft(0.0),
            sharp(-0.31, Y::Top(1.5)).heft(0.0),
        ]),
        Line::open(&[
            sharp(ONE_STEM - SERIF_REACH, FOOT),
            sharp(ONE_STEM + SERIF_REACH, FOOT),
        ]),
    ],
    &[Line::open(&[
        sharp(0.0, Y::Share(0.7)).heft(0.3),
        turn(0.0, TOP),
        turn(1.0, TOP),
        bowl(1.0, Y::Share(0.5)),
        sharp(0.0, FOOT).heft(0.2),
        sharp(1.0, FOOT),
    ])],
    &[
        Line::open(&[
            sharp(0.0, TOP),
            sharp(1.0, TOP),
            sharp(0.1, Y::Share(0.55)).heft(0.2),
        ]),
        Line::open(&[
            sharp(0.1, Y::Share(0.55)),
            turn(1.0, Y::Share(0.55)),
            sharp(1.0, Y::Foot(TURN)),
            turn(1.0, FOOT),
            turn(0.0, FOOT).heft(0.2),
            sharp(-0.05, Y::Share(0.2)).heft(0.0),
        ]),
    ],
    &[
        Line::open(&[sharp(0.38, Y::Brim), sharp(-0.43, Y::Share(BAR)).heft(0.0)])
            .cut(LEVEL, SQUARE),
        Line::open(&[sharp(-0.43, Y::Share(BAR)), sharp(1.43, Y::Share(BAR))]),
        Line::open(&[
            sharp(FOUR_STEM, Y::Share(0.645)),
            sharp(FOUR_STEM, Y::Foot(FLARE_RISE)),
            sharp(FOUR_STEM, Y::Base).heft(FLARE),
        ])
        .cut(LEVEL, LEVEL),
        Line::open(&[
            sharp(FOUR_STEM - SERIF_REACH, FOOT),
            sharp(FOUR_STEM + SERIF_REACH, FOOT),
        ]),
    ],
    &[Line::open(&[
        sharp(1.0, TOP),
        sharp(0.0, TOP),
        sharp(0.0, Y::Share(0.55)),
        turn(1.0, Y::Share(0.55)),
        sharp(1.0, Y::Foot(TURN)),
        turn(1.0, FOOT),
        turn(0.0, FOOT).heft(0.2),
        sharp(-0.05, Y::Share(0.2)).heft(0.0),
    ])],
    &[Line::open(&[
        sharp(0.95, Y::Share(0.8)).heft(0.0),
        turn(1.0, TOP).heft(0.0),
        turn(0.0, TOP),
        sharp(0.0, Y::Top(TURN)),
        sharp(0.0, Y::Foot(TURN)),
        turn(0.0, FOOT),
        turn(1.0, FOOT),
        sharp(1.0, Y::Foot(TURN)),
        turn(1.0, Y::Share(0.58)),
        turn(0.0, Y::Share(0.58)),
        sharp(0.0, Y::Share(0.3)),
    ])],
    &[Line::open(&[sharp(0.0, TOP), sharp(1.0, TOP), sharp(0.35, Y::Base)]).cut(SQUARE, LEVEL)],
    &[
        Line::closed(&[
            turn(0.0, FOOT),
            turn(1.0, FOOT),
            turn(1.0, Y::Share(0.52)),
            turn(0.0, Y::Share(0.52)),
        ]),
        Line::closed(&[
            turn(0.05, Y::Share(0.52)),
            turn(0.95, Y::Share(0.52)),
            turn(0.95, TOP),
            turn(0.05, TOP),
        ]),
    ],
    &[
        Line::closed(&[
            sharp(0.0, Y::Top(TURN)),
            turn(0.0, NINE_WAIST),
            turn(1.0, NINE_WAIST),
            sharp(1.0, Y::Top(TURN)),
            turn(1.0, TOP),
            turn(0.0, TOP),
        ]),
        Line::open(&[
            sharp(1.0, Y::Share(0.6)),
            sharp(1.0, Y::Foot(TURN)),
            bowl(1.0, FOOT),
            bowl(-0.12, FOOT).heft(0.0),
            sharp(-0.18, Y::Share(0.24)).heft(0.0),
        ]),
    ],
];

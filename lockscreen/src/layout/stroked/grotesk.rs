use super::{Style, sans};

pub(super) static STYLE: Style = Style {
    width: 0.31,
    bearing: 0.05,
    colon_bearing: 0.07,
    stem_gain: 1.0,
    contrast: 1.0,
    aspect: 1.3,
    dot_radius: 0.45,
    dot_spread: 0.218,
    digits: &sans::DIGITS,
};

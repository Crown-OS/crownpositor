//! The one spelling of an output mode: `"2560x1440@144.000"`, or
//! `"2560x1440"` for the fastest refresh at that size.

use smithay::{
    output::Mode,
    utils::{Physical, Size},
};

/// How far a requested refresh may be from an advertised one and still name
/// it. One hertz, in millihertz, because clients round millihertz differently.
pub const REFRESH_TOLERANCE_MILLIHERTZ: i32 = 1_000;

/// A mode as a user or a client asks for it: a size, and maybe a refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestedMode {
    pub size: Size<i32, Physical>,
    pub refresh_millihertz: Option<i32>,
}

/// Reads `"WxH"` or `"WxH@HZ"`. A malformed refresh rejects the whole string:
/// a typo must not quietly run a 144 Hz panel at 60.
pub fn parse(text: &str) -> Option<RequestedMode> {
    let (size, refresh) = match text.split_once('@') {
        Some((size, refresh)) => (size, Some(refresh)),
        None => (text, None),
    };
    let (width, height) = size.trim().split_once('x')?;
    let refresh_millihertz = match refresh {
        None => None,
        Some(hertz) => Some((hertz.trim().parse::<f64>().ok()? * 1000.0).round() as i32),
    };
    Some(RequestedMode {
        size: (width.trim().parse().ok()?, height.trim().parse().ok()?).into(),
        refresh_millihertz,
    })
}

pub fn format(mode: Mode) -> String {
    format!(
        "{}x{}@{:.3}",
        mode.size.w,
        mode.size.h,
        f64::from(mode.refresh) / 1000.0
    )
}

/// The index of the advertised mode a request names. Only an advertised size
/// counts; without a refresh the fastest one at that size is taken.
pub fn resolve(request: RequestedMode, advertised: &[Mode]) -> Option<usize> {
    let matching = advertised
        .iter()
        .enumerate()
        .filter(|(_, mode)| mode.size == request.size);
    match request.refresh_millihertz {
        None => matching
            .max_by_key(|(_, mode)| mode.refresh)
            .map(|(index, _)| index),
        Some(wanted) => matching
            .map(|(index, mode)| (index, (mode.refresh - wanted).abs()))
            .filter(|(_, distance)| *distance <= REFRESH_TOLERANCE_MILLIHERTZ)
            .min_by_key(|(_, distance)| *distance)
            .map(|(index, _)| index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: i32, height: i32, refresh: i32) -> Mode {
        Mode {
            size: (width, height).into(),
            refresh,
        }
    }

    fn advertised() -> Vec<Mode> {
        vec![
            mode(2560, 1440, 59_951),
            mode(2560, 1440, 143_998),
            mode(1920, 1080, 60_000),
        ]
    }

    fn request(width: i32, height: i32, refresh_millihertz: Option<i32>) -> RequestedMode {
        RequestedMode {
            size: (width, height).into(),
            refresh_millihertz,
        }
    }

    #[test]
    fn a_full_mode_string_parses() {
        assert_eq!(
            parse("2560x1440@144.000"),
            Some(request(2560, 1440, Some(144_000)))
        );
    }

    #[test]
    fn the_refresh_is_optional() {
        assert_eq!(parse("1920x1080"), Some(request(1920, 1080, None)));
    }

    #[test]
    fn whitespace_is_tolerated() {
        assert_eq!(
            parse(" 1280 x 720 @ 60 "),
            Some(request(1280, 720, Some(60_000)))
        );
    }

    #[test]
    fn nonsense_is_rejected_rather_than_guessed() {
        for bad in ["", "1920", "axb", "1920x", "x1080", "1920x1080@nope"] {
            assert_eq!(parse(bad), None, "{bad:?} must not parse");
        }
    }

    #[test]
    fn a_formatted_mode_parses_back_to_itself() {
        let original = mode(2560, 1440, 143_998);
        let text = format(original);
        assert_eq!(text, "2560x1440@143.998");
        let parsed = parse(&text);
        assert_eq!(parsed, Some(request(2560, 1440, Some(143_998))));
    }

    #[test]
    fn no_refresh_takes_the_fastest_mode_of_that_size() {
        assert_eq!(resolve(request(2560, 1440, None), &advertised()), Some(1));
    }

    #[test]
    fn a_refresh_snaps_to_the_nearest_within_a_hertz() {
        assert_eq!(
            resolve(request(2560, 1440, Some(60_000)), &advertised()),
            Some(0)
        );
        assert_eq!(
            resolve(request(2560, 1440, Some(144_000)), &advertised()),
            Some(1)
        );
    }

    #[test]
    fn a_refresh_further_than_a_hertz_names_nothing() {
        assert_eq!(
            resolve(request(2560, 1440, Some(75_000)), &advertised()),
            None
        );
    }

    #[test]
    fn a_size_the_output_does_not_advertise_names_nothing() {
        assert_eq!(resolve(request(3840, 2160, None), &advertised()), None);
    }
}

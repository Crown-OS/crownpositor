//! Picking a monitor's scale when the user has not.
//!
//! Scale used to be one system-wide setting, which is wrong the moment a
//! laptop is docked to a monitor of a different density: one of the two ends
//! up with text the wrong size. So each output gets its own, and an output
//! with nothing configured gets this guess.
//!
//! The heuristic is Mutter's, because it is the one every GNOME user's
//! hardware has already been judged against: aim for a *logical* density
//! rather than a fixed multiplier, with a lower target for large displays
//! because they are viewed from further away. So is its rule for which scales
//! to offer at all: only those that divide the panel into whole logical
//! pixels.

use smithay::utils::{Physical, Raw, Size};

/// The `wp_fractional_scale_v1` wire format is 120ths, so no other value can
/// be advertised without rounding behind the client's back.
const FRACTIONAL_DENOMINATOR: f64 = 120.0;

const MIN_SCALE: f64 = 1.0;
const MAX_SCALE: f64 = 4.0;
/// Quarter steps: the granularity users actually recognise, and what every
/// settings panel offers. Each is nudged within half a step to a scale that
/// divides the resolution evenly.
const STEP: f64 = 0.25;

/// Below this a scale is not offered, however dense the panel: it would leave
/// less usable area than a small tablet.
const MIN_LOGICAL_AREA: i32 = 800 * 480;

/// Diagonal inches at which a display stops being arm's-length.
const LARGE_MIN_DIAGONAL_INCHES: f64 = 20.0;
const MOBILE_TARGET_DPI: f64 = 135.0;
const LARGE_TARGET_DPI: f64 = 110.0;

/// The scale an output should start at, from its physical size and resolution.
///
/// Falls back to 1.0 when the panel does not report a size, which is common
/// for projectors and virtual outputs — guessing from a lie is worse than not
/// guessing.
pub fn guess_monitor_scale(size_mm: Size<i32, Raw>, resolution: Size<i32, Physical>) -> f64 {
    if size_mm.w <= 0 || size_mm.h <= 0 || resolution.w <= 0 || resolution.h <= 0 {
        return 1.0;
    }

    let diagonal_inches = diagonal(size_mm.w, size_mm.h) / 25.4;
    let target_dpi = if diagonal_inches < LARGE_MIN_DIAGONAL_INCHES {
        MOBILE_TARGET_DPI
    } else {
        LARGE_TARGET_DPI
    };
    let ideal = diagonal(resolution.w, resolution.h) / diagonal_inches / target_dpi;

    supported_scales(resolution)
        .min_by(|left, right| (left - ideal).abs().total_cmp(&(right - ideal).abs()))
        .unwrap_or(1.0)
}

/// The scales worth offering for a resolution, coarsest usable first.
///
/// A scale that leaves the logical size fractional — 1.75 makes a 4K panel
/// 2194.3 logical pixels wide — puts every window edge that touches the
/// screen's right or bottom between two physical pixels; 5/3 makes it 2304.
pub fn supported_scales(resolution: Size<i32, Physical>) -> impl Iterator<Item = f64> {
    let steps = ((MAX_SCALE - MIN_SCALE) / STEP) as i32;
    let mut scales: Vec<f64> = (0..=steps)
        .filter_map(|step| nearest_even_scale(resolution, MIN_SCALE + f64::from(step) * STEP))
        .filter(|scale| leaves_usable_area(resolution, *scale))
        .collect();
    scales.dedup();
    scales.into_iter()
}

/// The scale within half a step of `target` that divides `resolution` into
/// whole logical pixels and is nearest to it, if there is one.
fn nearest_even_scale(resolution: Size<i32, Physical>, target: f64) -> Option<f64> {
    let in_120ths = |scale: f64| (scale * FRACTIONAL_DENOMINATOR).round() as i64;
    (in_120ths(target - STEP / 2.0)..=in_120ths(target + STEP / 2.0))
        .filter(|numerator| divides_evenly(resolution, *numerator))
        .map(|numerator| numerator as f64 / FRACTIONAL_DENOMINATOR)
        .min_by(|left, right| (left - target).abs().total_cmp(&(right - target).abs()))
}

/// Whether `numerator`/120 divides `resolution` into whole logical pixels:
/// `width / (numerator / 120)` is whole exactly when `numerator` divides
/// `120 × width`.
fn divides_evenly(resolution: Size<i32, Physical>, numerator: i64) -> bool {
    let denominator = FRACTIONAL_DENOMINATOR as i64;
    [resolution.w, resolution.h]
        .into_iter()
        .all(|length| (i64::from(length) * denominator) % numerator == 0)
}

fn leaves_usable_area(resolution: Size<i32, Physical>, scale: f64) -> bool {
    let logical = resolution.to_f64().to_logical(scale).to_i32_round::<i32>();
    logical.w.saturating_mul(logical.h) >= MIN_LOGICAL_AREA
}

/// Rounds to a scale `wp_fractional_scale_v1` can actually name.
///
/// Advertising 1.31 when only 157/120 can be sent would have every client
/// render for a scale the compositor never asked for.
pub fn closest_representable_scale(scale: f64) -> f64 {
    (scale * FRACTIONAL_DENOMINATOR).round() / FRACTIONAL_DENOMINATOR
}

fn diagonal(width: i32, height: i32) -> f64 {
    f64::from(width).hypot(f64::from(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guess(size_mm: (i32, i32), resolution: (i32, i32)) -> f64 {
        guess_monitor_scale(size_mm.into(), resolution.into())
    }

    #[test]
    fn an_ordinary_desktop_monitor_stays_at_one() {
        // 24" 1080p, ~92 dpi.
        assert_eq!(guess((530, 300), (1920, 1080)), 1.0);
        // 27" 1440p, ~109 dpi.
        assert_eq!(guess((600, 340), (2560, 1440)), 1.0);
    }

    #[test]
    fn a_4k_27_inch_lands_on_one_and_a_half() {
        assert_eq!(guess((600, 340), (3840, 2160)), 1.5);
    }

    #[test]
    fn a_hidpi_laptop_panel_scales_up() {
        // 13.3" 2560x1600, ~227 dpi, viewed close: 1.68 ideally, and 5/3 is
        // the nearest scale that leaves it 1536x960 logical pixels.
        assert_eq!(guess((286, 179), (2560, 1600)), 200.0 / 120.0);
    }

    #[test]
    fn a_1080p_laptop_panel_stays_at_one() {
        // 15.6" 1080p, ~141 dpi.
        assert_eq!(guess((344, 194), (1920, 1080)), 1.0);
    }

    #[test]
    fn a_panel_that_reports_no_size_is_not_guessed_at() {
        assert_eq!(guess((0, 0), (1920, 1080)), 1.0);
        assert_eq!(guess((600, 340), (0, 0)), 1.0);
    }

    #[test]
    fn a_scale_that_would_leave_almost_no_desktop_is_not_offered() {
        // At 4x a 1080p panel is 480x270 logical, well under the floor.
        let offered: Vec<f64> = supported_scales((1920, 1080).into()).collect();
        assert!(offered.contains(&1.0));
        assert!(!offered.contains(&4.0));
    }

    #[test]
    fn every_offered_scale_leaves_whole_logical_pixels() {
        for resolution in [
            (1920, 1080),
            (1920, 1200),
            (2560, 1440),
            (2880, 1800),
            (3840, 2160),
        ] {
            for scale in supported_scales(resolution.into()) {
                let logical = (
                    f64::from(resolution.0) / scale,
                    f64::from(resolution.1) / scale,
                );
                assert_eq!(logical.0, logical.0.round(), "{resolution:?} at {scale}");
                assert_eq!(logical.1, logical.1.round(), "{resolution:?} at {scale}");
            }
        }
    }

    #[test]
    fn a_step_that_would_split_pixels_moves_to_one_that_does_not() {
        // 1.75 on 4K is 2194.3 logical pixels wide.
        let offered: Vec<f64> = supported_scales((3840, 2160).into()).collect();
        assert!(!offered.contains(&1.75));
        assert!(offered.contains(&(200.0 / 120.0)));
        assert!(offered.contains(&1.5));
        assert!(offered.contains(&2.0));
    }

    #[test]
    fn every_offered_scale_is_representable_on_the_wire() {
        for scale in supported_scales((3840, 2160).into()) {
            assert_eq!(closest_representable_scale(scale), scale);
        }
    }

    #[test]
    fn an_unrepresentable_scale_is_snapped_to_the_nearest_120th() {
        assert_eq!(closest_representable_scale(1.3), 156.0 / 120.0);
        assert!((closest_representable_scale(1.31) - 157.0 / 120.0).abs() < f64::EPSILON);
    }
}

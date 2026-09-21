//! Screen brightness and night light, as one gamma ramp per output.
//!
//! Both settings are a per-channel scaling of what is already in the
//! framebuffer, so the CRTC's lookup table is the right place for them: the
//! hardware does the lookup during scanout, nothing is re-rendered, and a
//! change costs one ioctl rather than a frame.
//!
//! This module is the arithmetic alone — it names no output and no backend.
//! Programming the result is [`State::apply_display_gamma`].
//!
//! [`State::apply_display_gamma`]: crate::state::State::apply_display_gamma

use config::Display;
use protocols::gamma_control::GammaRamps;

/// The colour temperature the warmth slider means at each end, in kelvin.
///
/// The cold end is D65, the white point every sRGB framebuffer is already
/// authored against, which is what makes warmth 0 exactly a no-op. The warm
/// end is roughly a halogen bulb — warmer than that stops reading as evening
/// light and starts reading as a broken monitor.
const NEUTRAL_KELVIN: f64 = 6500.0;
const WARMEST_KELVIN: f64 = 2500.0;

/// How dark the brightness slider is allowed to take a screen.
///
/// Zero would be a black screen with no way back, since the control that
/// undoes it is on that screen.
const MIN_BRIGHTNESS: f64 = 0.15;

/// The ramps for these settings, or `None` when they come to "leave it alone".
///
/// `None` is not the same as an identity ramp: it means the compositor has no
/// opinion, so a `zwlr_gamma_control_v1` client may keep the output.
pub fn ramps(size: u32, display: &Display) -> Option<GammaRamps> {
    let brightness = (display.brightness / 100.0).clamp(MIN_BRIGHTNESS, 1.0);
    let warmth = if display.night_light {
        (display.night_light_warmth / 100.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    if size == 0 || (brightness == 1.0 && warmth == 0.0) {
        return None;
    }

    let kelvin = NEUTRAL_KELVIN + warmth * (WARMEST_KELVIN - NEUTRAL_KELVIN);
    let [red, green, blue] = tint(kelvin).map(|channel| channel * brightness);

    Some(GammaRamps {
        red: channel_ramp(size, red),
        green: channel_ramp(size, green),
        blue: channel_ramp(size, blue),
    })
}

/// One channel's lookup table: a linear input scaled by `factor`.
fn channel_ramp(size: u32, factor: f64) -> Vec<u16> {
    let last = f64::from(size.saturating_sub(1)).max(1.0);
    (0..size)
        .map(|index| (f64::from(index) / last * factor * f64::from(u16::MAX)).round() as u16)
        .collect()
}

/// The per-channel scaling that moves a D65 white point to `kelvin`.
///
/// Normalised against D65 rather than used raw, so [`NEUTRAL_KELVIN`] is
/// exactly `[1, 1, 1]` — otherwise the approximation's own error would tint
/// the screen at warmth 0, where the user asked for nothing at all.
fn tint(kelvin: f64) -> [f64; 3] {
    let neutral = black_body(NEUTRAL_KELVIN);
    let wanted = black_body(kelvin);
    std::array::from_fn(|channel| (wanted[channel] / neutral[channel]).clamp(0.0, 1.0))
}

/// Tanner Helland's approximation of a black body's sRGB colour, in `0.0..=1.0`.
///
/// Accurate to a couple of percent across the range a night light uses, and it
/// is three logarithms rather than the interpolated CIE table a colorimeter
/// would want — the eye cannot tell the difference at these temperatures.
fn black_body(kelvin: f64) -> [f64; 3] {
    let t = (kelvin / 100.0).clamp(10.0, 400.0);

    let red = if t <= 66.0 {
        1.0
    } else {
        329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2) / 255.0
    };
    let green = if t <= 66.0 {
        (99.470_802_586 * t.ln() - 161.119_568_166) / 255.0
    } else {
        288.121_692_528 * (t - 60.0).powf(-0.075_514_849_2) / 255.0
    };
    let blue = if t >= 66.0 {
        1.0
    } else if t <= 19.0 {
        0.0
    } else {
        (138.517_731_223 * (t - 10.0).ln() - 305.044_792_730) / 255.0
    };

    [red, green, blue].map(|channel| channel.clamp(f64::MIN_POSITIVE, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: u32 = 256;

    fn display(brightness: f64, night_light: bool, warmth: f64) -> Display {
        Display {
            brightness,
            night_light,
            night_light_warmth: warmth,
        }
    }

    #[test]
    fn full_brightness_without_night_light_asks_for_nothing() {
        assert!(ramps(SIZE, &display(100.0, false, 50.0)).is_none());
        assert!(
            ramps(SIZE, &display(100.0, true, 0.0)).is_none(),
            "night light at zero warmth is night light doing nothing"
        );
    }

    #[test]
    fn an_output_with_no_programmable_gamma_gets_no_ramps() {
        assert!(ramps(0, &display(50.0, true, 50.0)).is_none());
    }

    #[test]
    fn brightness_scales_every_channel_alike() {
        let ramps = ramps(SIZE, &display(50.0, false, 0.0)).expect("dimming needs a ramp");

        assert_eq!(ramps.red, ramps.green);
        assert_eq!(ramps.green, ramps.blue);
        assert_eq!(ramps.red[0], 0, "black stays black");
        let full = f64::from(u16::MAX) * 0.5;
        assert!((f64::from(ramps.red[255]) - full).abs() < 1.0);
    }

    #[test]
    fn a_black_screen_is_not_reachable() {
        let ramps = ramps(SIZE, &display(0.0, false, 0.0)).expect("dimming needs a ramp");
        let peak = f64::from(ramps.red[255]) / f64::from(u16::MAX);
        assert!((peak - MIN_BRIGHTNESS).abs() < 0.01);
    }

    #[test]
    fn warmth_holds_red_back_and_takes_blue_away() {
        let ramps = ramps(SIZE, &display(100.0, true, 100.0)).expect("warmth needs a ramp");

        assert_eq!(ramps.red[255], u16::MAX, "the warm end never dims red");
        assert!(ramps.blue[255] < ramps.green[255]);
        assert!(ramps.green[255] < ramps.red[255]);
    }

    #[test]
    fn a_ramp_never_falls() {
        let ramps = ramps(SIZE, &display(70.0, true, 60.0)).expect("both need a ramp");

        for channel in [&ramps.red, &ramps.green, &ramps.blue] {
            assert_eq!(channel.len(), SIZE as usize);
            assert!(channel.windows(2).all(|pair| pair[0] <= pair[1]));
        }
    }

    #[test]
    fn the_neutral_temperature_tints_nothing() {
        assert_eq!(tint(NEUTRAL_KELVIN), [1.0, 1.0, 1.0]);
    }
}

//! Fitting light above reference white into an SDR output, on the CPU.
//!
//! The CPU reference the tests hold `color_convert.glsl` to — the shader does
//! the per-pixel work — so both must use the same constants. Light is measured
//! with `1.0` at reference white, and the output can show nothing above it.
//!
//! Below [`KNEE`] light passes untouched, so SDR content and the midtones of
//! HDR content keep their exact levels. Above it an extended Reinhard
//! shoulder, joined with matching slope, carries the content's peak onto the
//! output's peak. The curve works on the brightest channel and scales all
//! three by the same factor, which keeps hue and saturation.

/// Where the shoulder starts, as a fraction of reference white.
#[cfg(test)]
pub const KNEE: f64 = 0.75;

/// The output's encoding exponent: the `gamma22` the output describes itself
/// with, and the curve undescribed surfaces are already in.
pub const OUTPUT_GAMMA: f64 = 2.2;

/// `light` with its highlights compressed so `headroom` — the content's peak
/// over its reference white — lands on `1.0`.
#[cfg(test)]
pub fn tone_map(light: [f64; 3], headroom: f64) -> [f64; 3] {
    let brightest = light.into_iter().fold(0.0, f64::max);
    if headroom <= 1.0 || brightest <= KNEE {
        return light;
    }
    let gain = shoulder(brightest, headroom) / brightest;
    light.map(|channel| channel * gain)
}

#[cfg(test)]
fn shoulder(light: f64, headroom: f64) -> f64 {
    let span = 1.0 - KNEE;
    let over = (light - KNEE) / span;
    let white = (headroom - KNEE) / span;
    KNEE + span * over * (1.0 + over / (white * white)) / (1.0 + over)
}

/// Linear light in `[0, 1]` to the output's signal.
#[cfg(test)]
pub fn encode(light: f64) -> f64 {
    light.clamp(0.0, 1.0).powf(OUTPUT_GAMMA.recip())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PQ_HEADROOM: f64 = 10_000.0 / 203.0;

    fn mapped(light: f64, headroom: f64) -> f64 {
        tone_map([light; 3], headroom)[0]
    }

    #[test]
    fn content_without_headroom_is_untouched() {
        for light in [0.0, 0.3, 0.75, 0.9, 1.0] {
            assert_eq!(mapped(light, 1.0), light);
        }
    }

    #[test]
    fn light_below_the_knee_is_untouched_whatever_the_headroom() {
        for light in [0.0, 0.1, 0.5, KNEE] {
            assert_eq!(mapped(light, PQ_HEADROOM), light);
        }
    }

    #[test]
    fn the_content_peak_lands_on_the_output_peak() {
        for headroom in [1.01, 2.0, 4.93, PQ_HEADROOM] {
            let peak = mapped(headroom, headroom);
            assert!(
                (peak - 1.0).abs() < 1e-9,
                "headroom {headroom} should map its peak to 1.0, got {peak}"
            );
        }
    }

    #[test]
    fn the_shoulder_is_monotonic_and_never_exceeds_the_output() {
        let mut previous = 0.0;
        for step in 0..=1_000 {
            let light = f64::from(step) / 1_000.0 * PQ_HEADROOM;
            let out = mapped(light, PQ_HEADROOM);
            assert!(out >= previous, "the curve must not fold back at {light}");
            assert!(out <= 1.0 + 1e-9, "{light} overshot to {out}");
            previous = out;
        }
    }

    #[test]
    fn the_shoulder_joins_the_identity_with_matching_slope() {
        let epsilon = 1e-6;
        let slope = (mapped(KNEE + epsilon, PQ_HEADROOM) - KNEE) / epsilon;
        assert!(
            (slope - 1.0).abs() < 1e-3,
            "a kink at the knee shows as a band, slope was {slope}"
        );
    }

    #[test]
    fn hdr_reference_white_stays_bright() {
        let white = mapped(1.0, PQ_HEADROOM);
        assert!(
            (0.85..1.0).contains(&white),
            "diffuse white must stay near the top of SDR, got {white}"
        );
    }

    #[test]
    fn the_brightest_channel_sets_the_gain_so_hue_survives() {
        let [red, green, blue] = tone_map([4.0, 2.0, 1.0], PQ_HEADROOM);
        assert!((green / red - 0.5).abs() < 1e-12);
        assert!((blue / red - 0.25).abs() < 1e-12);
    }

    #[test]
    fn encoding_inverts_the_gamma22_curve() {
        for light in [0.0, 0.01, 0.18, 0.5, 1.0] {
            let signal = encode(light);
            assert!((signal.powf(OUTPUT_GAMMA) - light).abs() < 1e-12);
        }
        assert_eq!(encode(-0.5), 0.0);
        assert_eq!(encode(3.0), 1.0);
    }
}

//! Electro-optical transfer functions, on the CPU.
//!
//! These are the reference implementations. The shader has its own copies in
//! GLSL — it must, since this is per-pixel work — and the point of having both
//! is that the golden-image tests can compare the two. A transposed matrix or
//! a mistyped constant in a shader is invisible by eye and obvious against a
//! reference.
//!
//! # Units
//!
//! Each function matches its own specification rather than a house convention,
//! because that is what can be checked against published values:
//!
//! * relative curves map `[0, 1]` signal to `[0, 1]` display light,
//! * `st2084_pq` maps `[0, 1]` to absolute cd/m² in `[0, 10000]`,
//! * `hlg` maps `[0, 1]` to absolute cd/m² in `[0, Lw]`.
//!
//! Bringing those onto a common scale is [`super::pipeline`]'s job.

use protocols::color_management::record::{NamedTransferFunction, TransferFunction};

/// BT.2020 luma coefficients, which HLG's opto-optical transfer uses.
const BT2020_LUMA: [f64; 3] = [0.2627, 0.6780, 0.0593];

/// Signal to display light, per channel.
///
/// HLG is approximated here by its inverse OETF alone: its full definition is
/// cross-channel, so [`hlg_rgb`] is what a renderer must use. This scalar form
/// exists for the curve tests and for the shader's per-channel stage.
pub fn eotf(transfer: TransferFunction, signal: f64) -> f64 {
    match transfer {
        TransferFunction::Power(exponent) => {
            power(signal, f64::from(exponent) / 10_000.0)
        }
        TransferFunction::Named(named) => named_eotf(named, signal),
    }
}

fn named_eotf(transfer: NamedTransferFunction, signal: f64) -> f64 {
    use NamedTransferFunction as Tf;

    match transfer {
        Tf::ExtLinear => signal,
        Tf::Gamma22 => power(signal, 2.2),
        Tf::Gamma28 => power(signal, 2.8),
        // BT.1886 with a black level of zero, which is what a description
        // whose minimum luminance is zero asks for: L = V^2.4.
        Tf::Bt1886 => power(signal, 2.4),
        Tf::St240 => st240(signal),
        // The two sRGB spellings and the honestly-named one are the same
        // IEC 61966-2-1 curve; `ext_srgb` extends it through the origin.
        Tf::Srgb | Tf::CompoundPower2_4 => srgb(signal),
        Tf::ExtSrgb | Tf::Xvycc => mirrored(signal, srgb),
        Tf::St2084Pq => pq(signal),
        Tf::St428 => st428(signal),
        // Scene light; the display-light conversion needs every channel.
        Tf::Hlg => hlg_scene(signal),
    }
}

/// Display light back to a signal.
pub fn inverse_eotf(transfer: TransferFunction, light: f64) -> f64 {
    match transfer {
        TransferFunction::Power(exponent) => {
            power(light, 10_000.0 / f64::from(exponent))
        }
        TransferFunction::Named(named) => named_inverse_eotf(named, light),
    }
}

fn named_inverse_eotf(transfer: NamedTransferFunction, light: f64) -> f64 {
    use NamedTransferFunction as Tf;

    match transfer {
        Tf::ExtLinear => light,
        Tf::Gamma22 => power(light, 1.0 / 2.2),
        Tf::Gamma28 => power(light, 1.0 / 2.8),
        Tf::Bt1886 => power(light, 1.0 / 2.4),
        Tf::St240 => st240_inverse(light),
        Tf::Srgb | Tf::CompoundPower2_4 => srgb_inverse(light),
        Tf::ExtSrgb | Tf::Xvycc => mirrored(light, srgb_inverse),
        Tf::St2084Pq => pq_inverse(light),
        Tf::St428 => st428_inverse(light),
        Tf::Hlg => hlg_scene_inverse(light),
    }
}

/// A power curve mirrored through the origin.
///
/// The protocol defines `set_tf_power` this way so that a negative signal —
/// which a wide-gamut encoding can legitimately produce — stays negative
/// rather than becoming `NaN`.
fn power(value: f64, exponent: f64) -> f64 {
    value.abs().powf(exponent).copysign(value)
}

/// Extends a curve defined on non-negative values through the origin.
fn mirrored(value: f64, curve: fn(f64) -> f64) -> f64 {
    curve(value.abs()).copysign(value)
}

/// IEC 61966-2-1, the piecewise curve everyone calls sRGB.
fn srgb(signal: f64) -> f64 {
    if signal <= 0.040_45 {
        signal / 12.92
    } else {
        ((signal + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb_inverse(light: f64) -> f64 {
    if light <= 0.003_130_8 {
        light * 12.92
    } else {
        1.055 * light.powf(1.0 / 2.4) - 0.055
    }
}

/// SMPTE 240M.
fn st240(signal: f64) -> f64 {
    if signal < 0.0912 {
        signal / 4.0
    } else {
        ((signal + 0.1115) / 1.1115).powf(1.0 / 0.45)
    }
}

fn st240_inverse(light: f64) -> f64 {
    if light < 0.0228 {
        light * 4.0
    } else {
        1.1115 * light.powf(0.45) - 0.1115
    }
}

/// SMPTE ST 428-1, the digital-cinema curve. Peak is 48 cd/m².
fn st428(signal: f64) -> f64 {
    (52.37 / 48.0) * signal.max(0.0).powf(2.6)
}

fn st428_inverse(light: f64) -> f64 {
    (light.max(0.0) * 48.0 / 52.37).powf(1.0 / 2.6)
}

// SMPTE ST 2084 constants, as the ratios the standard states rather than as
// decimals, so they can be checked against it by eye.
const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;
/// The peak the curve is defined up to, in cd/m².
pub const PQ_PEAK: f64 = 10_000.0;

/// ST 2084 (PQ): signal to absolute cd/m².
fn pq(signal: f64) -> f64 {
    let signal = signal.clamp(0.0, 1.0);
    let encoded = signal.powf(1.0 / PQ_M2);
    let numerator = (encoded - PQ_C1).max(0.0);
    let denominator = PQ_C2 - PQ_C3 * encoded;
    if denominator <= 0.0 {
        return PQ_PEAK;
    }
    PQ_PEAK * (numerator / denominator).powf(1.0 / PQ_M1)
}

/// Absolute cd/m² back to a PQ signal.
fn pq_inverse(nits: f64) -> f64 {
    let normalized = (nits / PQ_PEAK).clamp(0.0, 1.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * normalized) / (1.0 + PQ_C3 * normalized)).powf(PQ_M2)
}

// ARIB STD-B67 / BT.2100 HLG constants, as the standard publishes them.
//
// `b` and `c` are stated rather than derived from `a` on purpose, so they can
// be checked against the standard by eye. Note that `a` itself is only given
// to eight digits, which leaves `hlg_scene(1.0)` about three parts in 10^8
// above one — inherent to the constants, not to this code, and far below the
// quantisation of any display.
const HLG_A: f64 = 0.178_832_77;
const HLG_B: f64 = 0.284_668_92;
const HLG_C: f64 = 0.559_910_73;

/// HLG's inverse OETF: signal to *scene* light in `[0, 1]`.
fn hlg_scene(signal: f64) -> f64 {
    let signal = signal.clamp(0.0, 1.0);
    if signal <= 0.5 {
        signal * signal / 3.0
    } else {
        (((signal - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
    }
}

fn hlg_scene_inverse(scene: f64) -> f64 {
    let scene = scene.clamp(0.0, 1.0);
    if scene <= 1.0 / 12.0 {
        (3.0 * scene).sqrt()
    } else {
        HLG_A * (12.0 * scene - HLG_B).ln() + HLG_C
    }
}

/// HLG's system gamma for a display of peak `reference_white` cd/m².
///
/// BT.2100: `γ = 1.2 + 0.42 · log10(Lw / 1000)`. A dimmer display gets a
/// lower gamma, which is what keeps HLG looking right across peak luminances
/// without any metadata.
pub fn hlg_system_gamma(reference_white: f64) -> f64 {
    1.2 + 0.42 * (reference_white.max(1.0) / 1000.0).log10()
}

/// The full HLG transfer: signal to absolute cd/m², all three channels.
///
/// Cross-channel by definition — the opto-optical transfer scales by the
/// *luminance* of the pixel, not by each channel — which is why the
/// per-channel [`eotf`] cannot do it alone.
pub fn hlg_rgb(signal: [f64; 3], peak_luminance: f64) -> [f64; 3] {
    let scene = signal.map(hlg_scene);
    let luma = scene[0] * BT2020_LUMA[0] + scene[1] * BT2020_LUMA[1] + scene[2] * BT2020_LUMA[2];
    let gamma = hlg_system_gamma(peak_luminance);
    let gain = peak_luminance * luma.max(0.0).powf(gamma - 1.0);

    scene.map(|channel| channel * gain)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVES: &[NamedTransferFunction] = &[
        NamedTransferFunction::Bt1886,
        NamedTransferFunction::Gamma22,
        NamedTransferFunction::Gamma28,
        NamedTransferFunction::St240,
        NamedTransferFunction::ExtLinear,
        NamedTransferFunction::Xvycc,
        NamedTransferFunction::Srgb,
        NamedTransferFunction::ExtSrgb,
        NamedTransferFunction::St2084Pq,
        NamedTransferFunction::St428,
        NamedTransferFunction::Hlg,
        NamedTransferFunction::CompoundPower2_4,
    ];

    #[test]
    fn every_curve_round_trips() {
        for curve in CURVES {
            for signal in [0.0, 0.01, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
                let transfer = TransferFunction::Named(*curve);
                let light = eotf(transfer, signal);
                let back = inverse_eotf(transfer, light);
                assert!(
                    (back - signal).abs() < 1e-6,
                    "{curve:?}: {signal} -> {light} -> {back}"
                );
            }
        }
    }

    #[test]
    fn a_power_curve_round_trips() {
        for exponent in [10_000u32, 18_000, 22_000, 26_000, 100_000] {
            let transfer = TransferFunction::Power(exponent);
            for signal in [0.0, 0.2, 0.5, 1.0] {
                let back = inverse_eotf(transfer, eotf(transfer, signal));
                assert!((back - signal).abs() < 1e-9, "exponent {exponent}");
            }
        }
    }

    #[test]
    fn every_curve_maps_black_to_black_and_white_to_its_peak() {
        for curve in CURVES {
            let transfer = TransferFunction::Named(*curve);
            assert!(
                eotf(transfer, 0.0).abs() < 1e-9,
                "{curve:?} does not map 0 to 0"
            );

            let white = eotf(transfer, 1.0);
            let expected = match curve {
                NamedTransferFunction::St2084Pq => PQ_PEAK,
                // Digital cinema's peak is above 1.0 by construction.
                NamedTransferFunction::St428 => 52.37 / 48.0,
                NamedTransferFunction::Hlg => 1.0,
                _ => 1.0,
            };
            assert!(
                (white - expected).abs() < 1e-6,
                "{curve:?}: white is {white}, expected {expected}"
            );
        }
    }

    #[test]
    fn srgb_matches_its_published_values() {
        // The knee, and the familiar mid-grey.
        assert!((srgb(0.040_45) - 0.003_130_8).abs() < 1e-7);
        assert!((srgb(0.5) - 0.214_041_14).abs() < 1e-6);
        assert!((srgb(1.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pq_matches_its_published_values() {
        // ST 2084's reference points: the curve is defined so that these land
        // on round numbers of nits.
        assert!((pq(1.0) - 10_000.0).abs() < 1e-6);
        assert!(pq(0.0).abs() < 1e-9);

        // 100 nits sits at roughly 0.508 signal; check by inverting.
        let signal = pq_inverse(100.0);
        assert!((pq(signal) - 100.0).abs() < 1e-6);
        assert!((signal - 0.508_078).abs() < 1e-4, "signal was {signal}");

        // And SDR reference white for HDR content.
        let reference = pq_inverse(203.0);
        assert!((pq(reference) - 203.0).abs() < 1e-6);
    }

    #[test]
    fn pq_is_monotonic() {
        let mut previous = -1.0;
        for step in 0..=1000 {
            let nits = pq(f64::from(step) / 1000.0);
            assert!(nits >= previous, "PQ must never decrease");
            previous = nits;
        }
    }

    #[test]
    fn hlg_system_gamma_follows_bt2100() {
        // The reference display: 1000 cd/m² gives exactly 1.2.
        assert!((hlg_system_gamma(1000.0) - 1.2).abs() < 1e-9);
        // Brighter displays get more gamma, dimmer ones less.
        assert!(hlg_system_gamma(4000.0) > 1.2);
        assert!(hlg_system_gamma(500.0) < 1.2);
    }

    #[test]
    fn hlg_white_reaches_the_displays_peak() {
        let peak = 1000.0;
        let white = hlg_rgb([1.0, 1.0, 1.0], peak);
        for channel in white {
            assert!(
                // Relative, and not tighter than the standard's own
                // constants allow — see the note by HLG_A.
                (channel - peak).abs() / peak < 1e-6,
                "HLG white must reach the peak, got {channel}"
            );
        }
    }

    #[test]
    fn hlg_black_stays_black() {
        for channel in hlg_rgb([0.0, 0.0, 0.0], 1000.0) {
            assert!(channel.abs() < 1e-9);
        }
    }

    #[test]
    fn the_mirrored_curves_pass_through_the_origin() {
        // xvYCC and extended sRGB must handle negative signals, which is the
        // whole reason they exist.
        for curve in [NamedTransferFunction::Xvycc, NamedTransferFunction::ExtSrgb] {
            let transfer = TransferFunction::Named(curve);
            let positive = eotf(transfer, 0.5);
            let negative = eotf(transfer, -0.5);
            assert!(
                (negative + positive).abs() < 1e-9,
                "{curve:?} must be odd about the origin"
            );
        }
    }

    #[test]
    fn a_power_curve_handles_negative_signal() {
        let transfer = TransferFunction::Power(22_000);
        assert!(eotf(transfer, -0.5) < 0.0, "a negative signal stays negative");
        assert!(!eotf(transfer, -0.5).is_nan());
    }

    #[test]
    fn st240_has_a_linear_toe_of_slope_one_quarter() {
        // The standard's rounded constants leave the two segments a few parts
        // in 10^5 apart at the knee, so the toe is what can be asserted
        // exactly; the round-trip test covers the pair being consistent.
        assert!((st240(0.08) - 0.02).abs() < 1e-12);
        assert!((st240_inverse(0.02) - 0.08).abs() < 1e-12);

        // And the discontinuity the standard itself carries stays small.
        let linear = 0.0912 / 4.0;
        let curved = ((0.0912 + 0.1115) / 1.1115f64).powf(1.0 / 0.45);
        assert!(
            (linear - curved).abs() < 1e-4,
            "the two segments must still meet to within the spec's rounding"
        );
    }

    #[test]
    fn bt1886_is_a_pure_two_point_four_power_at_zero_black() {
        let transfer = TransferFunction::Named(NamedTransferFunction::Bt1886);
        for signal in [0.1, 0.5, 0.9] {
            assert!((eotf(transfer, signal) - signal.powf(2.4)).abs() < 1e-9);
        }
    }
}

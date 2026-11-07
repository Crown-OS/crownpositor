//! Turning a surface's image description into shader uniforms.
//!
//! # The output encoding
//!
//! Outputs are driven 8-bit SDR, and the framebuffer holds what undescribed
//! surfaces are defined to be: sRGB primaries, a `gamma22` curve, `1.0` at
//! reference white. Undescribed surfaces are blended in it as they are, with
//! no conversion at all, and [`output_encoding`] is what the output and every
//! surface's preferred description report, so a well-behaved client renders
//! straight into it.
//!
//! A surface described as anything else is converted into that encoding as it
//! is drawn: decoded to linear light with its reference white on `1.0`, moved
//! onto the output's primaries, its highlights compressed into SDR by
//! [`super::tone_map`], and re-encoded. That is one fused shader per draw, not
//! an extra pass, and only for the surfaces that need it.
//!
//! Reference white as the unit because that is exactly the anchor the protocol
//! defines: content is "anchored" at its reference white, so a surface's
//! reference white must land on the output's whatever curve it arrived in.

use protocols::color_management::{
    math::{self, named},
    record::{
        DescriptionKind, ImageDescription, NamedPrimaries, NamedTransferFunction,
        ParametricDescription, PrimariesSpec, TransferFunction,
    },
};

use super::tone_map::OUTPUT_GAMMA;

/// Which curve the shader should decode with.
///
/// The values are shared with `color_convert.glsl`; changing one means
/// changing both, which is why they are written out rather than derived from
/// the enum's discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum TransferKind {
    Linear = 0,
    /// A pure power curve, exponent in [`SurfaceTransform::transfer_param`].
    Power = 1,
    /// IEC 61966-2-1, the piecewise curve.
    SrgbPiecewise = 2,
    /// The same, mirrored through the origin for negative signals.
    SrgbExtended = 3,
    St240 = 4,
    St2084Pq = 5,
    Hlg = 6,
    St428 = 7,
}

/// What one surface needs to reach the output encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceTransform {
    pub transfer: TransferKind,
    /// The power curve's exponent, or HLG's peak luminance in cd/m².
    pub transfer_param: f32,
    /// Multiplies decoded light so the source's reference white lands on 1.0.
    pub luminance_scale: f32,
    /// The content's peak over its reference white: what the tone map has to
    /// fit under 1.0.
    pub headroom: f32,
    /// Source primaries to the output's, column-major as GLSL wants a `mat3`.
    pub primaries: [[f32; 3]; 3],
}

const IDENTITY_MATRIX: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// How far a transform may stray from the output encoding and still be drawn
/// untouched: well under one 8-bit code anywhere on the curve.
const PASSTHROUGH_TOLERANCE: f32 = 1e-3;

/// The cd/m² a description's light is measured against.
#[derive(Debug, Clone, Copy)]
struct Levels {
    /// Where a relative curve's `1.0` lands.
    max: f64,
    reference: f64,
    /// The brightest the content gets.
    peak: f64,
}

/// What the output scans out, and so what every surface should render into.
pub fn output_encoding() -> DescriptionKind {
    let luminances = NamedTransferFunction::Gamma22.default_luminances();
    DescriptionKind::Parametric(ParametricDescription {
        primaries: PrimariesSpec::Named(NamedPrimaries::Srgb),
        transfer: TransferFunction::Named(NamedTransferFunction::Gamma22),
        luminances,
        target_primaries: named::SRGB,
        target_luminance: (luminances.min_scaled, luminances.max),
        max_cll: None,
        max_fall: None,
    })
}

impl SurfaceTransform {
    /// The transform for a surface with `description`.
    ///
    /// `None` for a description this compositor cannot decode — an ICC
    /// profile, which is refused at creation anyway.
    pub fn for_description(description: &ImageDescription) -> Option<Self> {
        match description.kind() {
            DescriptionKind::Parametric(parametric) => Some(Self::from_parts(
                parametric.primaries,
                parametric.transfer,
                parametric_levels(parametric),
            )),
            // scRGB: BT.709 primaries, linear, 1.0 at 80 cd/m² and 125.0 at
            // 10000, with the protocol's suggested reference white of 203.
            DescriptionKind::WindowsScrgb => Some(Self::from_parts(
                PrimariesSpec::Named(NamedPrimaries::Srgb),
                TransferFunction::Named(NamedTransferFunction::ExtLinear),
                Levels {
                    max: 80.0,
                    reference: 203.0,
                    peak: 10_000.0,
                },
            )),
            DescriptionKind::WindowsBt2100 => Some(Self::from_parts(
                PrimariesSpec::Named(NamedPrimaries::Bt2020),
                TransferFunction::Named(NamedTransferFunction::St2084Pq),
                Levels {
                    max: 10_000.0,
                    reference: 203.0,
                    peak: 10_000.0,
                },
            )),
            DescriptionKind::Icc(_) => None,
        }
    }

    fn from_parts(primaries: PrimariesSpec, transfer: TransferFunction, levels: Levels) -> Self {
        let (kind, param) = kind_of(transfer);
        let reference = levels.reference.max(1.0);

        // Absolute curves already produce cd/m², so the scale only has to
        // divide by the reference. A relative curve produces `[0, 1]` across
        // the description's own range, so it is scaled by that range first.
        //
        // The source's *black* level is deliberately not added back: it
        // describes the display the content was mastered for, and the output
        // has a black level of its own.
        let luminance_scale = match kind {
            TransferKind::St2084Pq | TransferKind::Hlg => reference.recip(),
            _ => levels.max / reference,
        };

        Self {
            transfer: kind,
            // HLG's system gamma depends on the peak it is shown at, which is
            // the description's maximum.
            transfer_param: match kind {
                TransferKind::Hlg => levels.max as f32,
                _ => param,
            },
            luminance_scale: luminance_scale as f32,
            headroom: (levels.peak / reference) as f32,
            primaries: to_output_primaries(primaries),
        }
    }

    /// Whether the surface is already in the output encoding, so it can be
    /// drawn exactly as an undescribed one is.
    pub fn is_passthrough(&self) -> bool {
        let close = |value: f32, expected: f32| (value - expected).abs() < PASSTHROUGH_TOLERANCE;
        self.transfer == TransferKind::Power
            && close(self.transfer_param, OUTPUT_GAMMA as f32)
            && close(self.luminance_scale, 1.0)
            && self.headroom <= 1.0 + PASSTHROUGH_TOLERANCE
            && self
                .primaries
                .iter()
                .flatten()
                .zip(IDENTITY_MATRIX.iter().flatten())
                .all(|(value, expected)| close(*value, *expected))
    }
}

/// The light levels of a parametric description. The peak is the brightest
/// the content claims to reach — its MaxCLL, else its mastering display's
/// peak — and never more than its encoding can carry.
fn parametric_levels(parametric: &ParametricDescription) -> Levels {
    let luminances = parametric.luminances;
    let mastered = Some(parametric.target_luminance.1).filter(|peak| *peak > 0);
    let claimed = parametric
        .max_cll
        .filter(|peak| *peak > 0)
        .or(mastered)
        .unwrap_or(luminances.max)
        .min(luminances.max);
    Levels {
        max: f64::from(luminances.max),
        reference: f64::from(luminances.reference),
        peak: f64::from(claimed),
    }
}

fn kind_of(transfer: TransferFunction) -> (TransferKind, f32) {
    use NamedTransferFunction as Tf;

    match transfer {
        TransferFunction::Power(exponent) => (TransferKind::Power, exponent as f32 / 10_000.0),
        TransferFunction::Named(named) => match named {
            Tf::ExtLinear => (TransferKind::Linear, 1.0),
            Tf::Gamma22 => (TransferKind::Power, 2.2),
            Tf::Gamma28 => (TransferKind::Power, 2.8),
            Tf::Bt1886 => (TransferKind::Power, 2.4),
            Tf::St240 => (TransferKind::St240, 1.0),
            Tf::Srgb | Tf::CompoundPower2_4 => (TransferKind::SrgbPiecewise, 1.0),
            Tf::ExtSrgb | Tf::Xvycc => (TransferKind::SrgbExtended, 1.0),
            Tf::St2084Pq => (TransferKind::St2084Pq, 1.0),
            Tf::St428 => (TransferKind::St428, 1.0),
            Tf::Hlg => (TransferKind::Hlg, 1000.0),
        },
    }
}

/// The matrix from `primaries` onto the output's sRGB primaries, adapted to
/// its D65 white.
///
/// Falls back to the identity when the volume is degenerate: a client that
/// sent nonsense should look untouched, not inverted.
fn to_output_primaries(primaries: PrimariesSpec) -> [[f32; 3]; 3] {
    convert_to_output(primaries).unwrap_or_else(|| {
        tracing::debug!(
            ?primaries,
            "degenerate primaries; leaving the colours alone"
        );
        IDENTITY_MATRIX
    })
}

fn convert_to_output(primaries: PrimariesSpec) -> Option<[[f32; 3]; 3]> {
    let source = primaries.to_xyz_matrix()?;
    let output = math::invert(math::rgb_to_xyz(named::SRGB)?)?;
    let source_white = primaries.resolve().white;
    let adapted = if source_white == named::SRGB.white {
        source
    } else {
        math::multiply(
            math::chromatic_adaptation(source_white, named::SRGB.white)?,
            source,
        )
    };
    let matrix = math::multiply(output, adapted);

    // GLSL's `mat3` is column-major, and the maths here is row-major.
    Some(std::array::from_fn(|column| {
        std::array::from_fn(|row| matrix[row][column] as f32)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::tone_map::{encode, tone_map};
    use protocols::color_management::{
        math::Primaries,
        record::{IccData, Luminances},
        registry::Registry,
    };

    fn transform(primaries: NamedPrimaries, transfer: NamedTransferFunction) -> SurfaceTransform {
        let luminances = transfer.default_luminances();
        SurfaceTransform::from_parts(
            PrimariesSpec::Named(primaries),
            TransferFunction::Named(transfer),
            Levels {
                max: f64::from(luminances.max),
                reference: f64::from(luminances.reference),
                peak: f64::from(luminances.max),
            },
        )
    }

    fn described(kind: DescriptionKind) -> SurfaceTransform {
        let description = Registry::new().intern(kind);
        SurfaceTransform::for_description(&description).expect("a transform")
    }

    /// Decodes and converts the way the shader does, on the CPU: linear light
    /// on the output's primaries, 1.0 at reference white, before tone mapping.
    fn apply(transform: SurfaceTransform, signal: [f64; 3]) -> [f64; 3] {
        let decoded = match transform.transfer {
            TransferKind::Hlg => {
                super::super::transfer::hlg_rgb(signal, f64::from(transform.transfer_param))
            }
            kind => signal.map(|channel| decode(kind, transform.transfer_param, channel)),
        };

        let scaled = decoded.map(|light| light * f64::from(transform.luminance_scale));

        // Column-major, as the shader multiplies it.
        std::array::from_fn(|row| {
            (0..3)
                .map(|column| f64::from(transform.primaries[column][row]) * scaled[column])
                .sum()
        })
    }

    /// The whole shader: what reaches the framebuffer.
    fn output_signal(transform: SurfaceTransform, signal: [f64; 3]) -> [f64; 3] {
        let light = apply(transform, signal).map(|channel| channel.max(0.0));
        tone_map(light, f64::from(transform.headroom)).map(encode)
    }

    fn decode(kind: TransferKind, param: f32, signal: f64) -> f64 {
        use super::super::transfer::{eotf, hlg_rgb};
        let transfer = match kind {
            TransferKind::Linear => TransferFunction::Named(NamedTransferFunction::ExtLinear),
            TransferKind::Power => {
                TransferFunction::Power((f64::from(param) * 10_000.0).round() as u32)
            }
            TransferKind::SrgbPiecewise => {
                TransferFunction::Named(NamedTransferFunction::CompoundPower2_4)
            }
            TransferKind::SrgbExtended => TransferFunction::Named(NamedTransferFunction::Xvycc),
            TransferKind::St240 => TransferFunction::Named(NamedTransferFunction::St240),
            TransferKind::St2084Pq => TransferFunction::Named(NamedTransferFunction::St2084Pq),
            TransferKind::St428 => TransferFunction::Named(NamedTransferFunction::St428),
            TransferKind::Hlg => return hlg_rgb([signal, signal, signal], f64::from(param))[0],
        };
        eotf(transfer, signal)
    }

    fn pq_signal(nits: f64) -> f64 {
        super::super::transfer::inverse_eotf(
            TransferFunction::Named(NamedTransferFunction::St2084Pq),
            nits,
        )
    }

    #[test]
    fn the_output_encoding_is_sdr_srgb() {
        let DescriptionKind::Parametric(output) = output_encoding() else {
            panic!("the output must be described parametrically");
        };
        assert_eq!(output.primaries, PrimariesSpec::Named(NamedPrimaries::Srgb));
        assert_eq!(
            output.transfer,
            TransferFunction::Named(NamedTransferFunction::Gamma22)
        );
        assert_eq!(output.luminances.max, output.luminances.reference);
        assert_eq!(output.target_luminance.1, output.luminances.reference);
        assert_eq!(output.target_primaries, named::SRGB);
    }

    #[test]
    fn a_surface_in_the_output_encoding_is_drawn_untouched() {
        assert!(described(output_encoding()).is_passthrough());
        assert!(transform(NamedPrimaries::Srgb, NamedTransferFunction::Gamma22).is_passthrough());
    }

    #[test]
    fn anything_else_is_converted() {
        for (primaries, transfer) in [
            (NamedPrimaries::Bt2020, NamedTransferFunction::St2084Pq),
            (NamedPrimaries::Bt2020, NamedTransferFunction::Hlg),
            (NamedPrimaries::DisplayP3, NamedTransferFunction::Gamma22),
            (
                NamedPrimaries::Srgb,
                NamedTransferFunction::CompoundPower2_4,
            ),
            (NamedPrimaries::Srgb, NamedTransferFunction::Bt1886),
            (NamedPrimaries::Srgb, NamedTransferFunction::ExtLinear),
        ] {
            assert!(
                !transform(primaries, transfer).is_passthrough(),
                "{primaries:?} {transfer:?} must go through the colour shader"
            );
        }
        assert!(!described(DescriptionKind::WindowsScrgb).is_passthrough());
        assert!(!described(DescriptionKind::WindowsBt2100).is_passthrough());
    }

    #[test]
    fn srgb_white_lands_on_reference_white() {
        let white = apply(
            transform(NamedPrimaries::Srgb, NamedTransferFunction::Gamma22),
            [1.0, 1.0, 1.0],
        );
        for channel in white {
            assert!((channel - 1.0).abs() < 1e-6, "got {channel}");
        }
    }

    #[test]
    fn pq_reference_white_also_lands_on_one() {
        // The protocol's anchoring rule: 203 cd/m² of PQ is diffuse white, and
        // must sit exactly where an SDR window's white sits before tone mapping.
        let transform = transform(NamedPrimaries::Bt2020, NamedTransferFunction::St2084Pq);
        let signal = pq_signal(203.0);
        for channel in apply(transform, [signal; 3]) {
            assert!(
                (channel - 1.0).abs() < 1e-4,
                "PQ reference white must be 1.0, got {channel}"
            );
        }
    }

    #[test]
    fn pq_headroom_is_the_peak_over_reference_white() {
        let transform = transform(NamedPrimaries::Bt2020, NamedTransferFunction::St2084Pq);
        assert!((f64::from(transform.headroom) - 10_000.0 / 203.0).abs() < 1e-3);
        assert!(apply(transform, [1.0; 3])[0] > 40.0);
    }

    #[test]
    fn mastering_metadata_narrows_the_headroom() {
        let luminances = NamedTransferFunction::St2084Pq.default_luminances();
        let hdr10 = |max_cll| {
            described(DescriptionKind::Parametric(ParametricDescription {
                primaries: PrimariesSpec::Named(NamedPrimaries::Bt2020),
                transfer: TransferFunction::Named(NamedTransferFunction::St2084Pq),
                luminances,
                target_primaries: named::DISPLAY_P3,
                target_luminance: (luminances.min_scaled, 1_000),
                max_cll,
                max_fall: None,
            }))
        };
        assert!((f64::from(hdr10(None).headroom) - 1_000.0 / 203.0).abs() < 1e-4);
        assert!((f64::from(hdr10(Some(600)).headroom) - 600.0 / 203.0).abs() < 1e-4);
    }

    #[test]
    fn pq_content_never_washes_out_on_the_sdr_output() {
        // The bug this pipeline exists for: PQ code values scanned out as if
        // they were gamma22. Reference white must come out near full white,
        // not the ~58% grey its PQ code value would show as.
        let transform = transform(NamedPrimaries::Bt2020, NamedTransferFunction::St2084Pq);
        let reference_signal = pq_signal(203.0);
        let white = output_signal(transform, [reference_signal; 3]);
        assert!(reference_signal < 0.6);
        for channel in white {
            assert!(channel > 0.9, "reference white came out at {channel}");
        }

        let peak = output_signal(transform, [1.0; 3]);
        for channel in peak {
            assert!((channel - 1.0).abs() < 1e-6, "the peak must reach 1.0");
        }

        // A dark grey stays dark rather than being lifted toward the PQ code.
        let shadow = output_signal(transform, [pq_signal(5.0); 3]);
        assert!(shadow[0] < 0.2, "5 cd/m² came out at {}", shadow[0]);
    }

    #[test]
    fn hlg_reference_white_lands_on_one() {
        let transform = transform(NamedPrimaries::Bt2020, NamedTransferFunction::Hlg);
        // HLG's reference white is at 75% signal on a 1000 nit display.
        let white = apply(transform, [0.75, 0.75, 0.75]);
        assert!(
            (white[0] - 1.0).abs() < 0.05,
            "HLG diffuse white should be near 1.0, got {}",
            white[0]
        );
    }

    #[test]
    fn scrgb_one_is_eighty_nits_under_a_203_nit_reference() {
        let transform = described(DescriptionKind::WindowsScrgb);
        let one = apply(transform, [1.0; 3]);
        assert!((one[0] - 80.0 / 203.0).abs() < 1e-5);
        assert!((f64::from(transform.headroom) - 10_000.0 / 203.0).abs() < 1e-3);
    }

    #[test]
    fn white_stays_neutral_through_the_primaries_matrix() {
        for primaries in [
            NamedPrimaries::Srgb,
            NamedPrimaries::DisplayP3,
            NamedPrimaries::Bt2020,
            NamedPrimaries::AdobeRgb,
            NamedPrimaries::DciP3,
        ] {
            let white = apply(
                transform(primaries, NamedTransferFunction::Gamma22),
                [1.0, 1.0, 1.0],
            );
            for channel in white {
                assert!(
                    (channel - 1.0).abs() < 1e-3,
                    "{primaries:?}: white must stay neutral white, got {white:?}"
                );
            }
        }
    }

    #[test]
    fn bt2020_red_falls_outside_srgb_and_is_clipped() {
        let transform = transform(NamedPrimaries::Bt2020, NamedTransferFunction::Gamma22);
        let red = apply(transform, [1.0, 0.0, 0.0]);
        assert!(red[0] > 1.0, "BT.2020 red is more saturated than sRGB red");
        assert!(red[1] < 0.0 && red[2] < 0.0);

        let shown = output_signal(transform, [1.0, 0.0, 0.0]);
        assert!(shown.iter().all(|channel| (0.0..=1.0).contains(channel)));
    }

    #[test]
    fn the_bt2020_to_srgb_matrix_matches_bt2087() {
        // ITU-R BT.2087's published BT.2020 to BT.709 matrix, row by row.
        let expected = [
            [1.6605, -0.5876, -0.0728],
            [-0.1246, 1.1329, -0.0083],
            [-0.0182, -0.1006, 1.1187],
        ];
        let matrix = transform(NamedPrimaries::Bt2020, NamedTransferFunction::ExtLinear).primaries;
        for (row, values) in expected.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                assert!(
                    (f64::from(matrix[column][row]) - value).abs() < 1e-3,
                    "[{row}][{column}] was {}, BT.2087 says {value}",
                    matrix[column][row]
                );
            }
        }
    }

    #[test]
    fn srgb_content_needs_no_primaries_conversion() {
        let transform = transform(NamedPrimaries::Srgb, NamedTransferFunction::ExtLinear);
        let identity = IDENTITY_MATRIX.iter().flatten();
        for (value, expected) in transform.primaries.iter().flatten().zip(identity) {
            assert!(
                (value - expected).abs() < 1e-5,
                "sRGB to sRGB must be the identity"
            );
        }
    }

    #[test]
    fn xyz_content_is_converted_rather_than_left_alone() {
        let transform = transform(NamedPrimaries::Cie1931Xyz, NamedTransferFunction::St428);
        assert_ne!(transform.primaries, IDENTITY_MATRIX);
    }

    #[test]
    fn black_stays_black_for_every_curve() {
        for transfer in [
            NamedTransferFunction::Gamma22,
            NamedTransferFunction::St2084Pq,
            NamedTransferFunction::Hlg,
            NamedTransferFunction::CompoundPower2_4,
            NamedTransferFunction::ExtLinear,
        ] {
            let black = output_signal(transform(NamedPrimaries::Bt2020, transfer), [0.0; 3]);
            for channel in black {
                assert!(channel.abs() < 1e-6, "{transfer:?} must map black to black");
            }
        }
    }

    #[test]
    fn an_icc_description_has_no_transform() {
        let description = Registry::new().intern(DescriptionKind::Icc(IccData::new(vec![1, 2, 3])));
        assert_eq!(SurfaceTransform::for_description(&description), None);
    }

    #[test]
    fn a_parametric_description_becomes_a_transform() {
        let transform = described(DescriptionKind::Parametric(ParametricDescription {
            primaries: PrimariesSpec::Named(NamedPrimaries::DisplayP3),
            transfer: TransferFunction::Named(NamedTransferFunction::Gamma22),
            luminances: Luminances {
                min_scaled: 2_000,
                max: 80,
                reference: 80,
            },
            target_primaries: named::SRGB,
            target_luminance: (2_000, 80),
            max_cll: None,
            max_fall: None,
        }));
        assert_eq!(transform.transfer, TransferKind::Power);
        assert!((transform.transfer_param - 2.2).abs() < 1e-6);
        assert!((transform.luminance_scale - 1.0).abs() < 1e-6);
        assert!((transform.headroom - 1.0).abs() < 1e-6);
        assert!(
            !transform.is_passthrough(),
            "P3 primaries still need converting"
        );
    }

    #[test]
    fn degenerate_primaries_fall_back_to_leaving_colours_alone() {
        let degenerate = Primaries {
            red: math::Chromaticity::new(0.3, 0.3),
            green: math::Chromaticity::new(0.4, 0.4),
            blue: math::Chromaticity::new(0.5, 0.5),
            white: math::Chromaticity::new(0.3127, 0.3290),
        };
        assert_eq!(
            to_output_primaries(PrimariesSpec::Raw(degenerate)),
            IDENTITY_MATRIX
        );
    }
}

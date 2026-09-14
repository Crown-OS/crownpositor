//! Turning a surface's image description into shader uniforms.
//!
//! # The compositing space
//!
//! BT.2020 primaries, linear light, with `1.0` meaning *the output's reference
//! white*. Every surface is brought into it before anything is blended, and
//! the output pass takes it back out again.
//!
//! BT.2020 because it contains every real display gamut, so gamut mapping
//! happens once on the way out rather than per surface, and because values
//! stay non-negative — which the alternative, BT.709-primaried scRGB, cannot
//! promise.
//!
//! Reference white as the unit because that is exactly the anchor the protocol
//! defines: content is "anchored" at its reference white, so a surface's
//! reference white must land on the output's whatever curve it arrived in.
//! That single rule is the whole of SDR-to-HDR handling — an SDR window on an
//! HDR screen is diffuse white, not a searchlight.

use protocols::color_management::{
    math::{self, Primaries, named},
    record::{
        DescriptionKind, ImageDescription, NamedTransferFunction, PrimariesSpec, TransferFunction,
    },
};

/// Which curve the shader should decode with.
///
/// The values are shared with `color_window.frag`; changing one means changing
/// both, which is why they are written out rather than derived from the enum's
/// discriminant.
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

/// What one surface needs to reach the compositing space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceTransform {
    pub transfer: TransferKind,
    /// The power curve's exponent, or HLG's peak luminance in cd/m².
    pub transfer_param: f32,
    /// Multiplies decoded light so the source's reference white lands on 1.0.
    pub luminance_scale: f32,
    /// Source primaries to BT.2020, column-major as GLSL wants a `mat3`.
    pub primaries: [[f32; 3]; 3],
}

/// The transform for content that is already in the compositing space.
const IDENTITY_MATRIX: [[f32; 3]; 3] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
];

impl SurfaceTransform {
    /// What an undescribed surface gets.
    ///
    /// The protocol defines this exactly: sRGB primaries, `gamma22`, at the
    /// reference white — so a legacy client, a cursor and a rasterised
    /// titlebar all take the same path as a client that said so explicitly.
    pub fn default_srgb() -> Self {
        Self::from_parts(
            PrimariesSpec::Named(
                protocols::color_management::record::NamedPrimaries::Srgb,
            ),
            TransferFunction::Named(NamedTransferFunction::Gamma22),
            80.0,
            80.0,
        )
    }

    /// The transform for a surface with `description`.
    ///
    /// `None` for a description this compositor cannot decode — an ICC
    /// profile, for now — which the caller must treat as a reason not to draw
    /// the surface wrong rather than a reason to drop it.
    pub fn for_description(description: &ImageDescription) -> Option<Self> {
        match description.kind() {
            DescriptionKind::Parametric(parametric) => Some(Self::from_parts(
                parametric.primaries,
                parametric.transfer,
                f64::from(parametric.luminances.max),
                f64::from(parametric.luminances.reference),
            )),
            // scRGB: BT.709 primaries, linear, with 1.0 at 80 cd/m².
            DescriptionKind::WindowsScrgb => Some(Self::from_parts(
                PrimariesSpec::Raw(named::SRGB),
                TransferFunction::Named(NamedTransferFunction::ExtLinear),
                80.0,
                80.0,
            )),
            DescriptionKind::WindowsBt2100 => Some(Self::from_parts(
                PrimariesSpec::Raw(named::BT2020),
                TransferFunction::Named(NamedTransferFunction::St2084Pq),
                10_000.0,
                203.0,
            )),
            DescriptionKind::Icc(_) => None,
        }
    }

    fn from_parts(
        primaries: PrimariesSpec,
        transfer: TransferFunction,
        max_luminance: f64,
        reference_luminance: f64,
    ) -> Self {
        let (kind, param) = kind_of(transfer);
        let reference = reference_luminance.max(1.0);

        // Absolute curves already produce cd/m², so the scale only has to
        // divide by the reference. A relative curve produces `[0, 1]` across
        // the description's own range, so it is scaled by that range first.
        //
        // The source's *black* level is deliberately not added back: it
        // describes the display the content was mastered for, and the output
        // has a black level of its own. Lifting by both would wash out
        // shadows twice over.
        let luminance_scale = match kind {
            TransferKind::St2084Pq | TransferKind::Hlg => 1.0 / reference,
            _ => max_luminance / reference,
        };

        Self {
            transfer: kind,
            // HLG's system gamma depends on the peak it is shown at, which is
            // the description's maximum.
            transfer_param: match kind {
                TransferKind::Hlg => max_luminance as f32,
                _ => param,
            },
            luminance_scale: luminance_scale as f32,
            primaries: to_bt2020(primaries),
        }
    }

    /// Whether this transform would change nothing.
    ///
    /// The renderer uses it to keep a surface that is already in the
    /// compositing space off the slow path even when the output is not.
    pub fn is_identity(&self) -> bool {
        self.transfer == TransferKind::Linear
            && (self.luminance_scale - 1.0).abs() < f32::EPSILON
            && self.primaries == IDENTITY_MATRIX
    }
}

fn kind_of(transfer: TransferFunction) -> (TransferKind, f32) {
    use NamedTransferFunction as Tf;

    match transfer {
        TransferFunction::Power(exponent) => {
            (TransferKind::Power, exponent as f32 / 10_000.0)
        }
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

/// The matrix from `primaries` into the compositing space.
///
/// Falls back to the identity when the volume is degenerate: a panel that
/// reported nonsense should look untouched, not inverted.
fn to_bt2020(primaries: PrimariesSpec) -> [[f32; 3]; 3] {
    let source = primaries.resolve();
    let Some(matrix) = convert_to_bt2020(source) else {
        tracing::debug!(?source, "degenerate primaries; leaving the colours alone");
        return IDENTITY_MATRIX;
    };
    matrix
}

fn convert_to_bt2020(source: Primaries) -> Option<[[f32; 3]; 3]> {
    let matrix = math::convert(source, named::BT2020, true)?;

    // GLSL's `mat3` is column-major, and the maths here is row-major.
    Some(std::array::from_fn(|column| {
        std::array::from_fn(|row| matrix[row][column] as f32)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocols::color_management::record::{
        Luminances, NamedPrimaries, ParametricDescription,
    };

    fn transform(
        primaries: NamedPrimaries,
        transfer: NamedTransferFunction,
    ) -> SurfaceTransform {
        let luminances = transfer.default_luminances();
        SurfaceTransform::from_parts(
            PrimariesSpec::Named(primaries),
            TransferFunction::Named(transfer),
            f64::from(luminances.max),
            f64::from(luminances.reference),
        )
    }

    /// Applies a transform the way the shader will, on the CPU.
    fn apply(transform: SurfaceTransform, signal: [f64; 3]) -> [f64; 3] {
        let decoded = match transform.transfer {
            TransferKind::Hlg => super::super::transfer::hlg_rgb(
                signal,
                f64::from(transform.transfer_param),
            ),
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

    #[test]
    fn srgb_white_lands_on_reference_white() {
        let white = apply(SurfaceTransform::default_srgb(), [1.0, 1.0, 1.0]);
        for channel in white {
            assert!(
                (channel - 1.0).abs() < 1e-6,
                "sRGB white must be 1.0 in the compositing space, got {channel}"
            );
        }
    }

    #[test]
    fn pq_reference_white_also_lands_on_one() {
        // The protocol's anchoring rule: 203 cd/m² of PQ is diffuse white, and
        // must sit exactly where an SDR window's white sits.
        let transform = transform(NamedPrimaries::Bt2020, NamedTransferFunction::St2084Pq);
        let signal = super::super::transfer::inverse_eotf(
            TransferFunction::Named(NamedTransferFunction::St2084Pq),
            203.0,
        );

        let white = apply(transform, [signal, signal, signal]);
        for channel in white {
            assert!(
                (channel - 1.0).abs() < 1e-4,
                "PQ reference white must be 1.0, got {channel}"
            );
        }
    }

    #[test]
    fn pq_peak_is_far_above_reference_white() {
        // 10000 nits against a 203 nit reference: HDR highlights must survive
        // into the compositing space rather than clipping at one.
        let transform = transform(NamedPrimaries::Bt2020, NamedTransferFunction::St2084Pq);
        let peak = apply(transform, [1.0, 1.0, 1.0]);
        assert!(
            peak[0] > 40.0,
            "PQ peak should be ~10000/203 = 49, got {}",
            peak[0]
        );
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
    fn white_stays_neutral_through_the_primaries_matrix() {
        for primaries in [
            NamedPrimaries::Srgb,
            NamedPrimaries::DisplayP3,
            NamedPrimaries::Bt2020,
            NamedPrimaries::AdobeRgb,
        ] {
            let white = apply(
                transform(primaries, NamedTransferFunction::Gamma22),
                [1.0, 1.0, 1.0],
            );
            assert!(
                (white[0] - white[1]).abs() < 1e-4 && (white[1] - white[2]).abs() < 1e-4,
                "{primaries:?}: white must stay neutral, got {white:?}"
            );
        }
    }

    #[test]
    fn srgb_content_fits_inside_the_compositing_space() {
        let transform = SurfaceTransform::default_srgb();
        for corner in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            for channel in apply(transform, corner) {
                assert!(
                    channel > -1e-4,
                    "sRGB is inside BT.2020, so nothing may go negative"
                );
            }
        }
    }

    #[test]
    fn bt2020_content_needs_no_primaries_conversion() {
        // Derived in f64 and stored as f32, so this is the identity to within
        // rounding rather than exactly.
        let transform = transform(NamedPrimaries::Bt2020, NamedTransferFunction::ExtLinear);
        for column in 0..3 {
            for row in 0..3 {
                let expected = IDENTITY_MATRIX[column][row];
                assert!(
                    (transform.primaries[column][row] - expected).abs() < 1e-5,
                    "BT.2020 to BT.2020 must be the identity, [{column}][{row}] was {}",
                    transform.primaries[column][row]
                );
            }
        }
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
            let black = apply(transform(NamedPrimaries::Bt2020, transfer), [0.0; 3]);
            for channel in black {
                assert!(channel.abs() < 1e-6, "{transfer:?} must map black to black");
            }
        }
    }

    #[test]
    fn an_icc_description_has_no_transform_yet() {
        use protocols::color_management::record::IccData;
        let mut registry = protocols::color_management::registry::Registry::new();
        let description = registry.intern(DescriptionKind::Icc(IccData::new(vec![1, 2, 3])));
        assert_eq!(SurfaceTransform::for_description(&description), None);
    }

    #[test]
    fn a_parametric_description_becomes_a_transform() {
        let mut registry = protocols::color_management::registry::Registry::new();
        let description = registry.intern(DescriptionKind::Parametric(ParametricDescription {
            primaries: PrimariesSpec::Named(NamedPrimaries::DisplayP3),
            transfer: TransferFunction::Named(NamedTransferFunction::Gamma22),
            luminances: Luminances { min_scaled: 2_000, max: 80, reference: 80 },
            target_primaries: named::SRGB,
            target_luminance: (2_000, 80),
            max_cll: None,
            max_fall: None,
        }));

        let transform = SurfaceTransform::for_description(&description).expect("a transform");
        assert_eq!(transform.transfer, TransferKind::Power);
        assert!((transform.transfer_param - 2.2).abs() < 1e-6);
        assert!((transform.luminance_scale - 1.0).abs() < 1e-6);
    }

    #[test]
    fn degenerate_primaries_fall_back_to_leaving_colours_alone() {
        let degenerate = Primaries {
            red: math::Chromaticity::new(0.3, 0.3),
            green: math::Chromaticity::new(0.4, 0.4),
            blue: math::Chromaticity::new(0.5, 0.5),
            white: math::Chromaticity::new(0.3127, 0.3290),
        };
        assert_eq!(to_bt2020(PrimariesSpec::Raw(degenerate)), IDENTITY_MATRIX);
    }
}

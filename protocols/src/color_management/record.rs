//! What an image description *is*, once the client has finished building it.
//!
//! Immutable by construction: a `wp_image_description_v1` that is `ready` can
//! never change, and the protocol leans on that — two objects describing the
//! same colours are required to report the same identity, and the information
//! read back from one must be byte-for-byte what it was the first time.

use std::num::NonZeroU64;

use super::math::{Chromaticity, Primaries, named};

/// Chromaticities travel as millionths on the wire.
pub const CHROMATICITY_SCALE: f64 = 1_000_000.0;
/// Minimum luminance travels as ten-thousandths of a cd/m².
pub const MIN_LUMINANCE_SCALE: f64 = 10_000.0;
/// Transfer-function exponents travel as ten-thousandths.
pub const POWER_SCALE: f64 = 10_000.0;

/// An image description's identity.
///
/// Monotonic and never recycled: the protocol lets clients compare these to
/// decide whether two descriptions are interchangeable, and reusing an id
/// after its description is gone would make two different colour volumes
/// compare equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DescriptionId(NonZeroU64);

impl DescriptionId {
    pub(super) fn new(raw: NonZeroU64) -> Self {
        Self(raw)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }

    /// The 32-bit form the deprecated `ready` event carries.
    ///
    /// Truncation is what the protocol asks for: version 1 clients are told
    /// explicitly that identities may be recycled.
    pub fn truncated(self) -> u32 {
        self.0.get() as u32
    }

    /// `(high, low)`, the two halves the version 2 events carry.
    pub fn halves(self) -> (u32, u32) {
        let raw = self.0.get();
        ((raw >> 32) as u32, raw as u32)
    }
}

/// The transfer functions this compositor is willing to name.
///
/// `log_100` and `log_316` are deliberately absent: both are undefined below
/// their toe, essentially nothing uses them, and advertising one would commit
/// us to inventing a mapping for the undefined region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedTransferFunction {
    Bt1886,
    Gamma22,
    Gamma28,
    St240,
    ExtLinear,
    Xvycc,
    /// Deprecated since version 2, so only advertised to version 1 clients.
    Srgb,
    /// Deprecated since version 2.
    ExtSrgb,
    St2084Pq,
    St428,
    Hlg,
    /// Since version 2. The IEC 61966-2-1 piecewise curve, named honestly.
    CompoundPower2_4,
}

impl NamedTransferFunction {
    /// The version at which this entry stops being advertised, if any.
    pub fn deprecated_since(self) -> Option<u32> {
        matches!(self, Self::Srgb | Self::ExtSrgb).then_some(2)
    }

    /// The version this entry first appears in.
    pub fn since(self) -> u32 {
        match self {
            Self::CompoundPower2_4 => 2,
            _ => 1,
        }
    }

    /// The luminance defaults the protocol prescribes for this curve.
    ///
    /// Not a compositor preference — the XML states these, and a client that
    /// omits `set_luminances` is entitled to exactly them.
    pub fn default_luminances(self) -> Luminances {
        match self {
            Self::St2084Pq => Luminances {
                min_scaled: 50,
                max: 10_000,
                reference: 203,
            },
            Self::Hlg => Luminances {
                min_scaled: 50,
                max: 1_000,
                reference: 203,
            },
            Self::Bt1886 => Luminances {
                min_scaled: 100,
                max: 100,
                reference: 100,
            },
            _ => Luminances {
                min_scaled: 2_000,
                max: 80,
                reference: 80,
            },
        }
    }

    /// Whether the curve encodes absolute luminance rather than a relative
    /// signal. PQ is the only one, and it is why PQ content needs no scaling
    /// to reach a known number of nits.
    pub fn is_absolute(self) -> bool {
        matches!(self, Self::St2084Pq)
    }
}

/// How the light-to-signal relationship is described.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferFunction {
    Named(NamedTransferFunction),
    /// A pure power curve, exponent in ten-thousandths.
    Power(u32),
}

/// The colour volume, either by name or by coordinates.
///
/// Kept apart even when they describe the same volume, because the
/// information read back differs: a named set also reports its name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PrimariesSpec {
    Named(NamedPrimaries),
    Raw(Primaries),
}

impl PrimariesSpec {
    pub fn resolve(self) -> Primaries {
        match self {
            Self::Named(named) => named.primaries(),
            Self::Raw(raw) => raw,
        }
    }

    /// The matrix taking this volume's linear RGB to CIE XYZ.
    pub fn to_xyz_matrix(self) -> Option<super::math::Matrix3> {
        match self {
            Self::Named(named) => named.to_xyz_matrix(),
            Self::Raw(raw) => super::math::rgb_to_xyz(raw),
        }
    }
}

macro_rules! named_primaries {
    ($($variant:ident => $value:expr),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum NamedPrimaries {
            $($variant,)*
        }

        impl NamedPrimaries {
            pub fn primaries(self) -> Primaries {
                match self {
                    $(Self::$variant => $value,)*
                }
            }

            /// Every named set, for the capability burst.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The matrix to CIE XYZ.
            ///
            /// XYZ itself is the identity: its "primaries" are the XYZ axes,
            /// whose chromaticities are degenerate by construction — X and Z
            /// both have `y == 0` — so the general derivation cannot be used
            /// even though those coordinates are exactly what the protocol
            /// wants reported for it.
            pub fn to_xyz_matrix(self) -> Option<$crate::color_management::math::Matrix3> {
                if matches!(self, Self::Cie1931Xyz) {
                    return Some([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
                }
                $crate::color_management::math::rgb_to_xyz(self.primaries())
            }
        }
    };
}

named_primaries! {
    Srgb => named::SRGB,
    PalM => named::PAL_M,
    Pal => named::PAL,
    Ntsc => named::NTSC,
    GenericFilm => named::GENERIC_FILM,
    Bt2020 => named::BT2020,
    Cie1931Xyz => named::CIE1931_XYZ,
    DciP3 => named::DCI_P3,
    DisplayP3 => named::DISPLAY_P3,
    AdobeRgb => named::ADOBE_RGB,
}

/// The luminance range a description is mastered for.
///
/// `min_scaled` keeps the wire's ten-thousandths so that two descriptions
/// built from the same numbers compare equal — rounding through `f64` would
/// make interning miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Luminances {
    pub min_scaled: u32,
    pub max: u32,
    pub reference: u32,
}

impl Luminances {
    pub fn min(&self) -> f64 {
        f64::from(self.min_scaled) / MIN_LUMINANCE_SCALE
    }

    /// Whether this range could describe a real display.
    pub fn is_valid(&self) -> bool {
        self.min() < f64::from(self.max)
            && self.reference > 0
            && self.reference <= self.max
            && f64::from(self.reference) > self.min()
    }
}

/// An ICC profile, as bytes plus the identity used to intern it.
#[derive(Debug, Clone)]
pub struct IccData {
    pub bytes: std::sync::Arc<[u8]>,
    hash: u64,
}

impl IccData {
    pub fn new(bytes: Vec<u8>) -> Self {
        let hash = fnv1a(&bytes);
        Self {
            bytes: bytes.into(),
            hash,
        }
    }
}

impl PartialEq for IccData {
    fn eq(&self, other: &Self) -> bool {
        // Hash first, then the bytes: two profiles that collide must still be
        // told apart, and the comparison is only reached on a collision.
        self.hash == other.hash && self.bytes == other.bytes
    }
}

impl Eq for IccData {}

impl std::hash::Hash for IccData {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.hash.hash(hasher);
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

/// A description built from parameters rather than a profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ParametricDescription {
    pub primaries: PrimariesSpec,
    pub transfer: TransferFunction,
    pub luminances: Luminances,
    /// The volume the content was mastered for. Defaults to the primaries.
    pub target_primaries: Primaries,
    /// `(min_scaled, max)`, defaulting to the description's own luminances.
    pub target_luminance: (u32, u32),
    pub max_cll: Option<u32>,
    pub max_fall: Option<u32>,
}

/// What kind of description this is, which decides what can be read back.
#[derive(Debug, Clone, PartialEq)]
pub enum DescriptionKind {
    Parametric(ParametricDescription),
    Icc(IccData),
    /// Windows' scRGB: BT.709 primaries, linear, with a fixed reference.
    WindowsScrgb,
    /// Windows' BT.2100 flavour.
    WindowsBt2100,
}

impl DescriptionKind {
    /// The colour volume this describes, where that is knowable without
    /// parsing an ICC profile.
    pub fn primaries(&self) -> Option<Primaries> {
        match self {
            Self::Parametric(parametric) => Some(parametric.primaries.resolve()),
            Self::WindowsScrgb => Some(named::SRGB),
            Self::WindowsBt2100 => Some(named::BT2020),
            Self::Icc(_) => None,
        }
    }

    /// Whether `get_information` may be answered for this kind.
    ///
    /// The Windows flavours refuse: they are defined by reference to another
    /// platform's behaviour rather than by parameters this protocol can name.
    pub fn is_describable(&self) -> bool {
        matches!(self, Self::Parametric(_) | Self::Icc(_))
    }
}

/// One finished, immutable image description.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageDescription {
    id: DescriptionId,
    kind: DescriptionKind,
    min_version: u32,
}

impl ImageDescription {
    pub(super) fn new(id: DescriptionId, kind: DescriptionKind) -> Self {
        let min_version = minimum_version(&kind);
        Self {
            id,
            kind,
            min_version,
        }
    }

    pub fn id(&self) -> DescriptionId {
        self.id
    }

    pub fn kind(&self) -> &DescriptionKind {
        &self.kind
    }

    /// The lowest `wp_image_description_v1` version that can carry this.
    ///
    /// Handing a description to an older client that cannot express it is
    /// what `failed(low_version)` exists for.
    pub fn min_version(&self) -> u32 {
        self.min_version
    }

    pub fn parametric(&self) -> Option<&ParametricDescription> {
        match &self.kind {
            DescriptionKind::Parametric(parametric) => Some(parametric),
            _ => None,
        }
    }
}

fn minimum_version(kind: &DescriptionKind) -> u32 {
    match kind {
        // `compound_power_2_4` did not exist before version 2, so a
        // description using it cannot be named to a version 1 client.
        DescriptionKind::Parametric(parametric)
            if parametric.transfer
                == TransferFunction::Named(NamedTransferFunction::CompoundPower2_4) =>
        {
            2
        }
        // `create_windows_bt2100` is a version 3 request.
        DescriptionKind::WindowsBt2100 => 3,
        _ => 1,
    }
}

/// Chromaticity from the wire's millionths.
pub fn chromaticity_from_wire(x: i32, y: i32) -> Chromaticity {
    Chromaticity::new(
        f64::from(x) / CHROMATICITY_SCALE,
        f64::from(y) / CHROMATICITY_SCALE,
    )
}

/// Chromaticity back to the wire's millionths.
pub fn chromaticity_to_wire(chromaticity: Chromaticity) -> (i32, i32) {
    (
        (chromaticity.x * CHROMATICITY_SCALE).round() as i32,
        (chromaticity.y * CHROMATICITY_SCALE).round() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_transfer_function_carries_the_protocols_own_defaults() {
        let pq = NamedTransferFunction::St2084Pq.default_luminances();
        assert_eq!((pq.min_scaled, pq.max, pq.reference), (50, 10_000, 203));

        let hlg = NamedTransferFunction::Hlg.default_luminances();
        assert_eq!((hlg.min_scaled, hlg.max, hlg.reference), (50, 1_000, 203));

        let bt1886 = NamedTransferFunction::Bt1886.default_luminances();
        assert_eq!((bt1886.min_scaled, bt1886.max, bt1886.reference), (100, 100, 100));

        let srgb = NamedTransferFunction::Gamma22.default_luminances();
        assert_eq!((srgb.min_scaled, srgb.max, srgb.reference), (2_000, 80, 80));
    }

    #[test]
    fn only_pq_encodes_absolute_luminance() {
        assert!(NamedTransferFunction::St2084Pq.is_absolute());
        assert!(!NamedTransferFunction::Hlg.is_absolute());
        assert!(!NamedTransferFunction::Gamma22.is_absolute());
    }

    #[test]
    fn the_deprecated_curves_are_the_two_srgb_spellings() {
        assert_eq!(NamedTransferFunction::Srgb.deprecated_since(), Some(2));
        assert_eq!(NamedTransferFunction::ExtSrgb.deprecated_since(), Some(2));
        assert_eq!(NamedTransferFunction::Gamma22.deprecated_since(), None);
        assert_eq!(NamedTransferFunction::CompoundPower2_4.since(), 2);
    }

    #[test]
    fn a_luminance_range_must_be_ordered_and_contain_its_reference() {
        assert!(Luminances { min_scaled: 2_000, max: 80, reference: 80 }.is_valid());
        assert!(!Luminances { min_scaled: 2_000, max: 0, reference: 80 }.is_valid());
        assert!(!Luminances { min_scaled: 2_000, max: 80, reference: 100 }.is_valid());
        assert!(!Luminances { min_scaled: 2_000, max: 80, reference: 0 }.is_valid());
    }

    #[test]
    fn a_description_using_a_version_2_curve_cannot_be_named_to_a_version_1_client() {
        let kind = DescriptionKind::Parametric(ParametricDescription {
            primaries: PrimariesSpec::Named(NamedPrimaries::Srgb),
            transfer: TransferFunction::Named(NamedTransferFunction::CompoundPower2_4),
            luminances: NamedTransferFunction::Gamma22.default_luminances(),
            target_primaries: named::SRGB,
            target_luminance: (2_000, 80),
            max_cll: None,
            max_fall: None,
        });
        assert_eq!(minimum_version(&kind), 2);
        assert_eq!(minimum_version(&DescriptionKind::WindowsBt2100), 3);
        assert_eq!(minimum_version(&DescriptionKind::WindowsScrgb), 1);
    }

    #[test]
    fn the_windows_flavours_refuse_to_describe_themselves() {
        assert!(!DescriptionKind::WindowsScrgb.is_describable());
        assert!(!DescriptionKind::WindowsBt2100.is_describable());
        assert!(DescriptionKind::Icc(IccData::new(vec![1, 2, 3])).is_describable());
    }

    #[test]
    fn identical_icc_bytes_compare_equal_and_different_ones_do_not() {
        assert_eq!(IccData::new(vec![1, 2, 3]), IccData::new(vec![1, 2, 3]));
        assert_ne!(IccData::new(vec![1, 2, 3]), IccData::new(vec![1, 2, 4]));
    }

    #[test]
    fn an_identity_splits_and_rejoins_across_the_two_wire_halves() {
        let id = DescriptionId::new(NonZeroU64::new(0x0000_0007_DEAD_BEEF).expect("nonzero"));
        let (high, low) = id.halves();
        assert_eq!(high, 7);
        assert_eq!(low, 0xDEAD_BEEF);
        assert_eq!((u64::from(high) << 32) | u64::from(low), id.get());
    }

    #[test]
    fn chromaticities_survive_the_wire_round_trip() {
        let white = Chromaticity::new(0.3127, 0.3290);
        let (x, y) = chromaticity_to_wire(white);
        assert_eq!((x, y), (312_700, 329_000));
        assert_eq!(chromaticity_from_wire(x, y), white);
    }

    #[test]
    fn every_named_primary_set_resolves_to_a_real_volume() {
        for named in NamedPrimaries::ALL {
            assert!(
                named.to_xyz_matrix().is_some(),
                "{named:?} must describe a volume"
            );
        }
    }

    #[test]
    fn cie_xyz_is_the_identity_despite_degenerate_chromaticities() {
        let matrix = NamedPrimaries::Cie1931Xyz
            .to_xyz_matrix()
            .expect("XYZ is a volume");
        assert_eq!(matrix, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        // The general derivation genuinely cannot do it, which is why the
        // special case exists rather than being an optimisation.
        assert_eq!(
            super::super::math::rgb_to_xyz(NamedPrimaries::Cie1931Xyz.primaries()),
            None
        );
    }
}

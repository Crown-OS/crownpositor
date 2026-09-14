//! What this compositor tells clients it can do, per bound version.
//!
//! The protocol is explicit that a compositor "must not advertise [entries]
//! that are deprecated in the bound version", and two entries are deprecated
//! since version 2 while two others only appear at 2 and 3. So the capability
//! burst is a function of the version, and keeping that in one table is what
//! makes a future version 4 a one-file change.
//!
//! Everything listed here is something the compositor will actually honour.
//! Advertising a feature and then producing implementation-defined mush is
//! worse for a client than a clean `failed(unsupported)`.

use super::record::{NamedPrimaries, NamedTransferFunction};

/// The render intents this compositor implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderIntent {
    Perceptual,
    Relative,
    Saturation,
    Absolute,
    RelativeBpc,
    /// Since version 2.
    AbsoluteNoAdaptation,
}

impl RenderIntent {
    fn since(self) -> u32 {
        match self {
            Self::AbsoluteNoAdaptation => 2,
            _ => 1,
        }
    }
}

/// The optional features of the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    IccV2V4,
    Parametric,
    SetPrimaries,
    SetTfPower,
    SetLuminances,
    SetMasteringDisplayPrimaries,
    ExtendedTargetVolume,
    WindowsScrgb,
    /// Gates the version 3 `create_windows_bt2100` request.
    WindowsBt2100,
}

impl Feature {
    fn since(self) -> u32 {
        match self {
            Self::WindowsBt2100 => 3,
            _ => 1,
        }
    }
}

/// Everything advertised on bind, already narrowed to one version.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub intents: Vec<RenderIntent>,
    pub features: Vec<Feature>,
    pub transfer_functions: Vec<NamedTransferFunction>,
    pub primaries: Vec<NamedPrimaries>,
}

/// The intents implemented.
///
/// `Saturation` is absent: it is a genuinely different gamut-mapping
/// algorithm aimed at business graphics, and this compositor does not
/// implement one. `Absolute` and `AbsoluteNoAdaptation` are absent until the
/// renderer can suppress chromatic adaptation and rescale to the source
/// white's absolute luminance.
const INTENTS: &[RenderIntent] = &[
    RenderIntent::Perceptual,
    RenderIntent::Relative,
    RenderIntent::RelativeBpc,
];

/// The features implemented.
///
/// `ExtendedTargetVolume` is absent until the tone mapper demonstrably handles
/// a target volume larger than the primary one; the protocol forbids
/// advertising it without `SetMasteringDisplayPrimaries` in any case.
const FEATURES: &[Feature] = &[
    Feature::Parametric,
    Feature::SetPrimaries,
    Feature::SetTfPower,
    Feature::SetLuminances,
    Feature::SetMasteringDisplayPrimaries,
    Feature::WindowsScrgb,
    Feature::WindowsBt2100,
];

/// The named curves implemented. See [`NamedTransferFunction`] for why the two
/// logarithmic curves are missing.
const TRANSFER_FUNCTIONS: &[NamedTransferFunction] = &[
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

impl Capabilities {
    /// What a client bound at `version` is told.
    pub fn for_version(version: u32) -> Self {
        Self {
            intents: INTENTS
                .iter()
                .copied()
                .filter(|intent| intent.since() <= version)
                .collect(),
            features: FEATURES
                .iter()
                .copied()
                .filter(|feature| feature.since() <= version)
                .collect(),
            transfer_functions: TRANSFER_FUNCTIONS
                .iter()
                .copied()
                .filter(|transfer| {
                    transfer.since() <= version
                        && transfer
                            .deprecated_since()
                            .is_none_or(|deprecated| version < deprecated)
                })
                .collect(),
            // Every named volume is a 3x3 matrix; there is nothing to be
            // dishonest about, and none are version-gated.
            primaries: NamedPrimaries::ALL.to_vec(),
        }
    }

    pub fn supports_intent(&self, intent: RenderIntent) -> bool {
        self.intents.contains(&intent)
    }

    pub fn supports_feature(&self, feature: Feature) -> bool {
        self.features.contains(&feature)
    }

    pub fn supports_transfer(&self, transfer: NamedTransferFunction) -> bool {
        self.transfer_functions.contains(&transfer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deprecated_srgb_curves_reach_version_one_only() {
        let v1 = Capabilities::for_version(1);
        assert!(v1.supports_transfer(NamedTransferFunction::Srgb));
        assert!(v1.supports_transfer(NamedTransferFunction::ExtSrgb));

        for version in [2, 3] {
            let capabilities = Capabilities::for_version(version);
            assert!(
                !capabilities.supports_transfer(NamedTransferFunction::Srgb),
                "srgb is deprecated since 2 and must not be advertised at {version}"
            );
            assert!(!capabilities.supports_transfer(NamedTransferFunction::ExtSrgb));
        }
    }

    #[test]
    fn compound_power_appears_only_from_version_two() {
        assert!(
            !Capabilities::for_version(1)
                .supports_transfer(NamedTransferFunction::CompoundPower2_4)
        );
        assert!(
            Capabilities::for_version(2)
                .supports_transfer(NamedTransferFunction::CompoundPower2_4)
        );
    }

    #[test]
    fn the_windows_bt2100_feature_appears_only_from_version_three() {
        assert!(!Capabilities::for_version(2).supports_feature(Feature::WindowsBt2100));
        assert!(Capabilities::for_version(3).supports_feature(Feature::WindowsBt2100));
        // scRGB has no such gate.
        assert!(Capabilities::for_version(1).supports_feature(Feature::WindowsScrgb));
    }

    #[test]
    fn absolute_no_adaptation_would_appear_only_from_version_two() {
        // Not implemented yet, so absent everywhere — but the gate is in place.
        assert_eq!(RenderIntent::AbsoluteNoAdaptation.since(), 2);
        assert!(
            !Capabilities::for_version(3).supports_intent(RenderIntent::AbsoluteNoAdaptation)
        );
    }

    #[test]
    fn perceptual_is_always_advertised() {
        // The protocol requires it; every version must offer it.
        for version in 1..=3 {
            assert!(
                Capabilities::for_version(version).supports_intent(RenderIntent::Perceptual),
                "perceptual is mandatory and missing at version {version}"
            );
        }
    }

    #[test]
    fn nothing_we_cannot_honour_is_advertised() {
        let capabilities = Capabilities::for_version(3);
        assert!(!capabilities.supports_intent(RenderIntent::Saturation));
        assert!(!capabilities.supports_feature(Feature::ExtendedTargetVolume));
        // But what we do implement is offered.
        assert!(capabilities.supports_transfer(NamedTransferFunction::Bt1886));
    }

    #[test]
    fn extended_target_volume_is_never_advertised_without_its_prerequisite() {
        // The protocol forbids the pairing; this holds trivially today but
        // would catch someone enabling the feature without the other.
        let capabilities = Capabilities::for_version(3);
        if capabilities.supports_feature(Feature::ExtendedTargetVolume) {
            assert!(capabilities.supports_feature(Feature::SetMasteringDisplayPrimaries));
        }
    }
}

//! Interning image descriptions, so equal colours share an identity.
//!
//! The protocol lets a client compare two descriptions' identities to decide
//! whether a surface needs reconfiguring, and requires that descriptions
//! "describing the same image description" share one. Two clients that arrive
//! at BT.2020 + PQ by different routes must therefore get the same id.
//!
//! A linear scan rather than a hash map: descriptions are counted in single
//! digits, and the keys contain chromaticities — floating point, which has no
//! sensible `Hash`. Comparing them is exact on purpose, because a client that
//! sent different numbers asked for a different description even if the
//! difference is invisible.

use std::{
    num::NonZeroU64,
    sync::{Arc, Weak},
};

use super::record::{DescriptionId, DescriptionKind, ImageDescription};

#[derive(Default)]
pub struct Registry {
    /// Only ever increments, including past descriptions that have been
    /// dropped: a recycled id would make two different colour volumes compare
    /// equal for any client that remembered the old one.
    next: u64,
    live: Vec<(DescriptionKind, Weak<ImageDescription>)>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The description for these colours, creating it only if it is new.
    pub fn intern(&mut self, kind: DescriptionKind) -> Arc<ImageDescription> {
        self.live.retain(|(_, weak)| weak.strong_count() > 0);

        if let Some(existing) = self
            .live
            .iter()
            .find(|(existing, _)| *existing == kind)
            .and_then(|(_, weak)| weak.upgrade())
        {
            return existing;
        }

        self.next += 1;
        let id = DescriptionId::new(
            NonZeroU64::new(self.next).expect("the counter starts at one and only grows"),
        );
        let description = Arc::new(ImageDescription::new(id, kind.clone()));
        self.live.push((kind, Arc::downgrade(&description)));
        description
    }

    /// How many descriptions are currently alive. For tests and diagnostics.
    pub fn live(&self) -> usize {
        self.live
            .iter()
            .filter(|(_, weak)| weak.strong_count() > 0)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color_management::{
        math::named,
        record::{
            Luminances, NamedPrimaries, NamedTransferFunction, ParametricDescription,
            PrimariesSpec, TransferFunction,
        },
    };

    fn parametric(transfer: NamedTransferFunction) -> DescriptionKind {
        DescriptionKind::Parametric(ParametricDescription {
            primaries: PrimariesSpec::Named(NamedPrimaries::Srgb),
            transfer: TransferFunction::Named(transfer),
            luminances: transfer.default_luminances(),
            target_primaries: named::SRGB,
            target_luminance: (2_000, 80),
            max_cll: None,
            max_fall: None,
        })
    }

    #[test]
    fn the_same_colours_intern_to_the_same_identity() {
        let mut registry = Registry::new();
        let first = registry.intern(parametric(NamedTransferFunction::Gamma22));
        let second = registry.intern(parametric(NamedTransferFunction::Gamma22));

        assert_eq!(first.id(), second.id());
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(registry.live(), 1);
    }

    #[test]
    fn different_colours_get_different_identities() {
        let mut registry = Registry::new();
        let srgb = registry.intern(parametric(NamedTransferFunction::Gamma22));
        let pq = registry.intern(parametric(NamedTransferFunction::St2084Pq));

        assert_ne!(srgb.id(), pq.id());
        assert_eq!(registry.live(), 2);
    }

    #[test]
    fn a_named_volume_and_the_same_one_written_out_stay_distinct() {
        // They describe identical colours, but the information read back
        // differs: only the named one reports a name.
        let mut registry = Registry::new();
        let named_srgb = registry.intern(parametric(NamedTransferFunction::Gamma22));

        let raw = DescriptionKind::Parametric(ParametricDescription {
            primaries: PrimariesSpec::Raw(named::SRGB),
            transfer: TransferFunction::Named(NamedTransferFunction::Gamma22),
            luminances: NamedTransferFunction::Gamma22.default_luminances(),
            target_primaries: named::SRGB,
            target_luminance: (2_000, 80),
            max_cll: None,
            max_fall: None,
        });
        assert_ne!(named_srgb.id(), registry.intern(raw).id());
    }

    #[test]
    fn an_identity_is_never_reused_after_its_description_is_dropped() {
        let mut registry = Registry::new();

        let dropped_id = {
            let description = registry.intern(parametric(NamedTransferFunction::Hlg));
            description.id()
        };

        // Interning the very same colours again, after the first is gone.
        let fresh = registry.intern(parametric(NamedTransferFunction::Hlg));
        assert_ne!(
            fresh.id(),
            dropped_id,
            "a released identity must not come back"
        );
    }

    #[test]
    fn dropping_a_description_frees_its_slot() {
        let mut registry = Registry::new();
        {
            let _held = registry.intern(parametric(NamedTransferFunction::Gamma22));
            assert_eq!(registry.live(), 1);
        }

        // The sweep happens on the next intern, so the dead entry goes and
        // only the new one is left.
        let _hlg = registry.intern(parametric(NamedTransferFunction::Hlg));
        assert_eq!(registry.live(), 1);
    }

    #[test]
    fn an_icc_profile_interns_on_its_bytes() {
        use crate::color_management::record::IccData;

        let mut registry = Registry::new();
        let first = registry.intern(DescriptionKind::Icc(IccData::new(vec![1, 2, 3])));
        let same = registry.intern(DescriptionKind::Icc(IccData::new(vec![1, 2, 3])));
        let other = registry.intern(DescriptionKind::Icc(IccData::new(vec![1, 2, 4])));

        assert_eq!(first.id(), same.id());
        assert_ne!(first.id(), other.id());
    }
}

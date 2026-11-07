//! The surfaces a frame converts into the output's encoding, by element.
//!
//! Keyed by the element id smithay gives a surface, which is all a decorator
//! sees of the surface it is drawing. Empty in the common case, where looking
//! a surface up costs no hashing at all.

use std::collections::HashMap;

use smithay::backend::renderer::element::Id;

use crate::color::pipeline::SurfaceTransform;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conversion {
    pub transform: SurfaceTransform,
    /// The description's identity: a new one repaints the whole surface, as
    /// its pixels change without the client damaging them.
    pub description: u64,
}

#[derive(Debug, Default)]
pub struct ColorConversions(HashMap<Id, Conversion>);

impl ColorConversions {
    pub fn get(&self, id: &Id) -> Option<Conversion> {
        self.0.get(id).copied()
    }
}

impl FromIterator<(Id, Conversion)> for ColorConversions {
    fn from_iter<I: IntoIterator<Item = (Id, Conversion)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::pipeline::TransferKind;

    fn conversion() -> Conversion {
        Conversion {
            transform: SurfaceTransform {
                transfer: TransferKind::St2084Pq,
                transfer_param: 1.0,
                luminance_scale: 1.0 / 203.0,
                headroom: 10_000.0 / 203.0,
                primaries: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
            description: 7,
        }
    }

    #[test]
    fn only_listed_surfaces_are_converted() {
        let listed = Id::new();
        let conversions: ColorConversions = [(listed.clone(), conversion())].into_iter().collect();
        assert_eq!(conversions.get(&listed), Some(conversion()));
        assert_eq!(conversions.get(&Id::new()), None);
    }

    #[test]
    fn an_empty_frame_converts_nothing() {
        assert_eq!(ColorConversions::default().get(&Id::new()), None);
    }
}

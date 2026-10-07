//! The identities the damage tracker knows the overview's own quads by.

use smithay::backend::renderer::element::Id;

/// The identities the damage tracker knows the overview's own quads by. They
/// outlive a frame: a fresh one every frame reads as a new element and
/// repaints the whole screen.
#[derive(Debug)]
pub struct OverviewIds {
    pub backdrop: Id,
    pub add: TileIds,
    previews: Vec<TileIds>,
}

impl Default for OverviewIds {
    fn default() -> Self {
        Self {
            backdrop: Id::new(),
            add: TileIds::default(),
            previews: Vec::new(),
        }
    }
}

impl OverviewIds {
    /// `None` for a preview the bar has not been laid out with yet.
    pub fn preview(&self, index: usize) -> Option<&TileIds> {
        self.previews.get(index)
    }

    /// Grows to `previews`, never renumbering the ones already on screen.
    pub(super) fn ensure(&mut self, previews: usize) {
        if self.previews.len() < previews {
            self.previews.resize_with(previews, TileIds::default);
        }
    }
}

/// One tile of the bar: its glass, its ring, and its glyph's quads — the '×'
/// disc on a preview, the '+' bars on the add tile.
#[derive(Debug)]
pub struct TileIds {
    pub glass: Id,
    pub ring: Id,
    pub glyph: [Id; 2],
}

impl Default for TileIds {
    fn default() -> Self {
        Self {
            glass: Id::new(),
            ring: Id::new(),
            glyph: [Id::new(), Id::new()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growing_the_bar_keeps_the_identities_already_handed_out() {
        let mut ids = OverviewIds::default();
        ids.ensure(1);
        let first = ids.preview(0).map(|tile| tile.glass.clone());
        ids.ensure(6);
        assert_eq!(ids.preview(0).map(|tile| tile.glass.clone()), first);
        assert!(ids.preview(5).is_some());
        assert!(ids.preview(6).is_none());
    }

    #[test]
    fn every_quad_has_its_own_identity() {
        let mut ids = OverviewIds::default();
        ids.ensure(2);
        let mut all = vec![ids.backdrop.clone()];
        for tile in [Some(&ids.add), ids.preview(0), ids.preview(1)]
            .into_iter()
            .flatten()
        {
            all.extend([tile.glass.clone(), tile.ring.clone()]);
            all.extend(tile.glyph.iter().cloned());
        }
        for (index, id) in all.iter().enumerate() {
            assert!(
                !all[index + 1..].contains(id),
                "two quads share an identity"
            );
        }
    }
}

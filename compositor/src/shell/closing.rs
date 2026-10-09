//! Windows whose client has gone, shrinking away from a snapshot of their last
//! frame.

use crate::{
    rendering::snapshot::Snapshot,
    shell::{morph::IconOrigin, tile::Tile},
};

/// A closed window on its way into its icon. Keeps its tile, so its frame is
/// drawn exactly as it was, and its last frame, since its surfaces are gone.
pub struct ClosingWindow {
    tile: Tile,
    snapshot: Snapshot,
}

impl ClosingWindow {
    pub fn new(mut tile: Tile, snapshot: Snapshot, origin: IconOrigin) -> Self {
        tile.shrink_into(origin);
        Self { tile, snapshot }
    }

    pub fn tile(&self) -> &Tile {
        &self.tile
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Advances the motion; returns whether it is still on screen.
    pub fn step(&mut self, dt: f32) -> bool {
        self.tile.anim_mut().step(dt);
        self.tile.step_morph(dt);
        self.tile.morph().is_some()
    }
}

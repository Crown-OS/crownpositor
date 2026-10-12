//! Windows whose client has gone, leaving from a snapshot of their last frame.

use crate::{
    rendering::snapshot::Snapshot,
    shell::{dust::Dust, morph::IconOrigin, tile::Tile},
};

/// How a closed window leaves the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Farewell {
    /// Shrinks into its icon, or away where it stood.
    Shrink(IconOrigin),
    /// Crumbles into falling dust.
    Dust(Dust),
}

/// A closed window on its way off screen. Keeps its tile, so its frame is
/// drawn exactly as it was, and its last frame, since its surfaces are gone.
pub struct ClosingWindow {
    tile: Tile,
    snapshot: Snapshot,
    dust: Option<Dust>,
}

impl ClosingWindow {
    pub fn new(mut tile: Tile, snapshot: Snapshot, farewell: Farewell) -> Self {
        let dust = match farewell {
            Farewell::Shrink(origin) => {
                tile.shrink_into(origin);
                None
            }
            Farewell::Dust(dust) => Some(dust),
        };
        Self {
            tile,
            snapshot,
            dust,
        }
    }

    pub fn tile(&self) -> &Tile {
        &self.tile
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn dust(&self) -> Option<&Dust> {
        self.dust.as_ref()
    }

    /// Advances the motion; returns whether it is still on screen.
    pub fn step(&mut self, dt: f32) -> bool {
        if let Some(dust) = &mut self.dust {
            return dust.step(dt);
        }
        self.tile.anim_mut().step(dt);
        self.tile.step_morph(dt);
        self.tile.morph().is_some()
    }
}

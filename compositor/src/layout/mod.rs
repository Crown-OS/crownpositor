//! Pure geometry.
//!
//! Nothing here can see a `WlSurface`, a `Window`, an `Output` or the `Shell`.
//! [`TilingLayout`] is handed a description of the tiled windows and hands back
//! one rectangle each; it cannot reorder the list, drop a window or send a
//! configure. What it does own is its own parameters — master ratio, master
//! count — and those survive the workspace switching to floating and back.
//!
//! The payoff is that the arithmetic is testable with a `Vec<TileInfo>` and a
//! rectangle, with no display and no event loop.

pub mod placement;
pub mod snap;
pub mod tiling;

use smithay::utils::{Logical, Rectangle, Size};

use crate::utils::id::WindowId;

/// Which regime a workspace arranges its windows under.
///
/// Owned outright by each workspace: the config value only seeds a workspace as
/// it is created, so switching one workspace never touches its neighbours. It is
/// the config's own type rather than a mirror of it, because a second two-variant
/// enum and the conversion between them would say nothing the first does not.
pub use config::WorkspaceMode;
pub use snap::{SnapBounds, SnapZone};
pub use tiling::TilingLayout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gaps {
    /// Between tiles.
    pub inner: i32,
    /// Between the tiled region and the edge of the usable area.
    pub outer: i32,
}

impl Default for Gaps {
    fn default() -> Self {
        Self { inner: 8, outer: 8 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Left,
    Right,
    Top,
    Bottom,
}

/// One tiled window, as much of it as the layout may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileInfo {
    pub id: WindowId,
    /// A zero component means unconstrained.
    pub min_size: Size<i32, Logical>,
    /// A zero component means unconstrained.
    pub max_size: Size<i32, Logical>,
}

impl TileInfo {
    pub fn new(id: WindowId) -> Self {
        Self {
            id,
            min_size: Size::default(),
            max_size: Size::default(),
        }
    }

    /// Clamps a proposed size into the window's own limits.
    pub fn constrain(&self, size: Size<i32, Logical>) -> Size<i32, Logical> {
        let clamp = |value: i32, min: i32, max: i32| {
            let value = if min > 0 { value.max(min) } else { value };
            if max > 0 { value.min(max) } else { value }
        };
        Size::from((
            clamp(size.w, self.min_size.w, self.max_size.w),
            clamp(size.h, self.min_size.h, self.max_size.h),
        ))
    }
}

/// Borrowed for exactly one call, rebuilt from the workspace each relayout.
#[derive(Debug)]
pub struct LayoutInput<'a> {
    /// Workspace-local (origin `0,0`), exclusive zones and the outer gap already
    /// subtracted. The layout never sees global coordinates, so one arrangement
    /// computed against eDP-1 works unchanged on HDMI-1.
    pub area: Rectangle<i32, Logical>,
    pub gaps: Gaps,
    pub focused: Option<WindowId>,
    /// Tiled windows in layout order. Never empty.
    pub tiles: &'a [TileInfo],
}

impl LayoutInput<'_> {
    pub fn index_of(&self, id: WindowId) -> Option<usize> {
        self.tiles.iter().position(|tile| tile.id == id)
    }
}

/// Owned by the workspace and reused, so a relayout allocates only when the tile
/// count passes its high-water mark.
#[derive(Debug, Default)]
pub struct LayoutOutput {
    /// Exactly one rect per input tile, in the same order. Workspace-local.
    pub rects: Vec<Rectangle<i32, Logical>>,
}

impl LayoutOutput {
    pub fn clear(&mut self) {
        self.rects.clear();
    }
}

/// Adjustments to the tiling parameters.
///
/// Note what is absent: swap, move-to-front, insert-at. Tile order belongs to
/// the workspace, which applies those itself; a layout that also owned the order
/// could disagree with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutOp {
    /// Grow or shrink the master column by a fraction of the area.
    Grow(f64),
    /// Move a window into or out of the master column.
    PromoteDemote(WindowId),
    ResetSize,
    DragEdge {
        edge: ResizeEdge,
        delta: f64,
    },
}

/// Splits `area` into `count` rows separated by `gap`, distributing the
/// remainder so the rows fill the area exactly.
pub(crate) fn split_rows(
    area: Rectangle<i32, Logical>,
    count: usize,
    gap: i32,
) -> Vec<Rectangle<i32, Logical>> {
    if count == 0 {
        return Vec::new();
    }

    let usable = (area.size.h - gap * (count as i32 - 1)).max(count as i32);
    let each = usable / count as i32;
    // Handing the remainder to the leading rows is what makes the rects cover
    // the area exactly instead of leaving a stripe of background.
    let remainder = usable % count as i32;

    let mut rects = Vec::with_capacity(count);
    let mut offset = 0;

    for index in 0..count as i32 {
        let extent = each + i32::from(index < remainder);
        rects.push(Rectangle::new(
            (area.loc.x, area.loc.y + offset).into(),
            (area.size.w, extent).into(),
        ));
        offset += extent + gap;
    }

    rects
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub fn area(w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), (w, h).into())
    }

    pub fn tiles(count: usize) -> Vec<TileInfo> {
        (0..count)
            .map(|_| TileInfo::new(WindowId::next()))
            .collect()
    }

    pub fn input<'a>(
        area: Rectangle<i32, Logical>,
        tiles: &'a [TileInfo],
        gaps: Gaps,
    ) -> LayoutInput<'a> {
        LayoutInput {
            area,
            gaps,
            focused: tiles.first().map(|tile| tile.id),
            tiles,
        }
    }

    /// The rects must cover the area on the split axis with exactly `gap`
    /// between them and nothing left over.
    pub fn assert_covers_vertically(
        rects: &[Rectangle<i32, Logical>],
        area: Rectangle<i32, Logical>,
        gap: i32,
    ) {
        assert_eq!(rects.first().unwrap().loc.y, area.loc.y);
        let last = rects.last().unwrap();
        assert_eq!(last.loc.y + last.size.h, area.loc.y + area.size.h);
        for pair in rects.windows(2) {
            assert_eq!(pair[1].loc.y - (pair[0].loc.y + pair[0].size.h), gap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::*, *};

    #[test]
    fn rows_fill_the_area_exactly() {
        // 100 does not divide by 3, so this is the remainder case.
        let rects = split_rows(area(200, 100), 3, 4);
        assert_covers_vertically(&rects, area(200, 100), 4);
    }

    #[test]
    fn a_single_row_takes_everything() {
        let rects = split_rows(area(200, 100), 1, 8);
        assert_eq!(rects, vec![area(200, 100)]);
    }

    #[test]
    fn splitting_survives_an_area_smaller_than_its_gaps() {
        // Every tile ends up 1px rather than zero or negative.
        let rects = split_rows(area(50, 4), 4, 8);
        assert_eq!(rects.len(), 4);
        assert!(rects.iter().all(|rect| rect.size.h >= 1));
    }

    #[test]
    fn constrain_respects_min_and_max() {
        let tile = TileInfo {
            id: WindowId::next(),
            min_size: (100, 0).into(),
            max_size: (0, 300).into(),
        };
        let size = tile.constrain((40, 900).into());
        assert_eq!(size.w, 100, "below min widens");
        assert_eq!(size.h, 300, "above max shrinks");
        // A zero component is unconstrained on that axis.
        assert_eq!(tile.constrain((500, 10).into()), (500, 10).into());
    }
}

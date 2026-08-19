//! Pure geometry.
//!
//! Nothing here can see a `WlSurface`, a `Window`, an `Output` or the `Shell`.
//! An algorithm is handed a description of the tiled windows and hands back one
//! rectangle each; it cannot reorder the list, drop a window or send a
//! configure. What it does own is its own parameters — master ratio, column
//! widths, scroll offset — and those survive being swapped out.
//!
//! The payoff is that every algorithm is testable with a `Vec<TileInfo>` and a
//! rectangle, with no display and no event loop.

pub mod floating;
pub mod master_stack;
pub mod scrolling;

use std::fmt::Debug;

use smithay::utils::{Logical, Point, Rectangle, Size};

use config::LayoutMode;

use crate::utils::id::WindowId;

pub use floating::NoTiling;
pub use master_stack::MasterStack;
pub use scrolling::ScrollingColumns;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LayoutKind {
    #[default]
    MasterStack,
    ScrollingColumns,
    Floating,
}

impl From<LayoutMode> for LayoutKind {
    fn from(mode: LayoutMode) -> Self {
        match mode {
            LayoutMode::MasterStack => Self::MasterStack,
            LayoutMode::ScrollingColumns => Self::ScrollingColumns,
            LayoutMode::Floating => Self::Floating,
        }
    }
}

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

/// One tiled window, as much of it as an algorithm may see.
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
    /// subtracted. Algorithms never see global coordinates, so one written
    /// against eDP-1 works unchanged on HDMI-1.
    pub area: Rectangle<i32, Logical>,
    pub gaps: Gaps,
    pub focused: Option<WindowId>,
    /// Tiled windows in layout order. Never empty.
    pub tiles: &'a [TileInfo],
}

impl<'a> LayoutInput<'a> {
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
    /// Applied at render and hit-test time rather than baked into `rects`, so a
    /// scrolling layout can pan without a relayout or retargeting every spring.
    pub view_offset: Point<f64, Logical>,
}

impl LayoutOutput {
    pub fn clear(&mut self) {
        self.rects.clear();
        self.view_offset = Point::default();
    }
}

/// Algorithm-specific adjustments.
///
/// Note what is absent: swap, move-to-front, insert-at. Tile order belongs to
/// the workspace, which applies those generically for every algorithm; an
/// algorithm that also owned the order could disagree with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutOp {
    /// Grow or shrink the primary split by a fraction of the area.
    Grow(f64),
    /// Move a window into or out of the master area.
    PromoteDemote(WindowId),
    /// Cycle a window through the algorithm's preset sizes.
    CyclePreset(WindowId),
    ResetSize(WindowId),
    DragEdge {
        id: WindowId,
        edge: ResizeEdge,
        delta: Point<f64, Logical>,
    },
}

pub trait LayoutAlgorithm: Debug + 'static {
    fn kind(&self) -> LayoutKind;

    /// Must push exactly `input.tiles.len()` rects, in input order.
    fn layout(&mut self, input: &LayoutInput<'_>, out: &mut LayoutOutput);

    /// `true` if anything changed, which is what sets the workspace's dirty bit.
    fn apply(&mut self, op: LayoutOp, input: &LayoutInput<'_>) -> bool;

    /// Drop per-window state. Called from the shell's single removal choke
    /// point, so an algorithm's side tables cannot outlive a window.
    fn forget(&mut self, id: WindowId);

    /// `None` falls back to the workspace's list-order neighbour.
    fn neighbour(
        &self,
        input: &LayoutInput<'_>,
        from: WindowId,
        dir: Direction,
    ) -> Option<WindowId> {
        let _ = (input, from, dir);
        None
    }

    /// Pan the viewport. Only viewport-style layouts implement this.
    fn scroll(&mut self, delta: Point<f64, Logical>, input: &LayoutInput<'_>) -> bool {
        let _ = (delta, input);
        false
    }

    /// Pan so `id` is fully visible, after a focus change or an insert.
    fn reveal(&mut self, id: WindowId, input: &LayoutInput<'_>) -> bool {
        let _ = (id, input);
        false
    }
}

/// Every algorithm a workspace has used, kept alive.
///
/// Toggling MasterStack -> Scrolling -> MasterStack restores the master ratio
/// you had set, and the scrolling layout still knows every column's width.
#[derive(Debug)]
pub struct LayoutSet {
    active: LayoutKind,
    /// At most three, so a `Vec` beats a `HashMap` on both counts.
    slots: Vec<Box<dyn LayoutAlgorithm>>,
}

impl LayoutSet {
    pub fn new(kind: LayoutKind) -> Self {
        Self {
            active: kind,
            slots: vec![instantiate(kind)],
        }
    }

    pub fn active(&self) -> LayoutKind {
        self.active
    }

    pub fn set_active(&mut self, kind: LayoutKind) -> bool {
        if self.active == kind {
            return false;
        }
        if !self.slots.iter().any(|slot| slot.kind() == kind) {
            self.slots.push(instantiate(kind));
        }
        self.active = kind;
        true
    }

    pub fn current(&self) -> &dyn LayoutAlgorithm {
        let kind = self.active;
        self.slots
            .iter()
            .find(|slot| slot.kind() == kind)
            .expect("the active layout is always instantiated")
            .as_ref()
    }

    pub fn current_mut(&mut self) -> &mut dyn LayoutAlgorithm {
        let kind = self.active;
        self.slots
            .iter_mut()
            .find(|slot| slot.kind() == kind)
            .expect("the active layout is always instantiated")
            .as_mut()
    }

    /// Forwarded to every slot, not just the active one — an inactive
    /// algorithm's side table must not retain a dead id either.
    pub fn forget(&mut self, id: WindowId) {
        for slot in &mut self.slots {
            slot.forget(id);
        }
    }
}

fn instantiate(kind: LayoutKind) -> Box<dyn LayoutAlgorithm> {
    match kind {
        LayoutKind::MasterStack => Box::new(MasterStack::default()),
        LayoutKind::ScrollingColumns => Box::new(ScrollingColumns::default()),
        LayoutKind::Floating => Box::new(NoTiling),
    }
}

/// Splits `area` into `count` rows separated by `gap`, distributing the
/// remainder so the rows fill the area exactly.
pub(crate) fn split_rows(
    area: Rectangle<i32, Logical>,
    count: usize,
    gap: i32,
) -> Vec<Rectangle<i32, Logical>> {
    split(area, count, gap, true)
}

fn split(
    area: Rectangle<i32, Logical>,
    count: usize,
    gap: i32,
    vertical: bool,
) -> Vec<Rectangle<i32, Logical>> {
    if count == 0 {
        return Vec::new();
    }

    let total = if vertical { area.size.h } else { area.size.w };
    let usable = (total - gap * (count as i32 - 1)).max(count as i32);
    let each = usable / count as i32;
    // Handing the remainder to the leading tiles is what makes the rects cover
    // the area exactly instead of leaving a stripe of background.
    let remainder = usable % count as i32;

    let mut rects = Vec::with_capacity(count);
    let mut offset = 0;

    for index in 0..count as i32 {
        let extent = each + i32::from(index < remainder);
        rects.push(if vertical {
            Rectangle::new(
                (area.loc.x, area.loc.y + offset).into(),
                (area.size.w, extent).into(),
            )
        } else {
            Rectangle::new(
                (area.loc.x + offset, area.loc.y).into(),
                (extent, area.size.h).into(),
            )
        });
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
        (0..count).map(|_| TileInfo::new(WindowId::next())).collect()
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

    #[test]
    fn switching_layouts_keeps_the_old_one_alive() {
        let mut set = LayoutSet::new(LayoutKind::MasterStack);
        let tiles = tiles(3);
        let input = input(area(800, 600), &tiles, Gaps::default());

        set.current_mut().apply(LayoutOp::Grow(0.15), &input);
        let mut grown = LayoutOutput::default();
        set.current_mut().layout(&input, &mut grown);

        set.set_active(LayoutKind::ScrollingColumns);
        set.set_active(LayoutKind::MasterStack);

        let mut again = LayoutOutput::default();
        set.current_mut().layout(&input, &mut again);
        assert_eq!(
            grown.rects, again.rects,
            "a round trip must not reset the master ratio"
        );
    }

    #[test]
    fn forget_reaches_inactive_slots() {
        let mut set = LayoutSet::new(LayoutKind::ScrollingColumns);
        let tiles = tiles(2);
        let input = input(area(800, 600), &tiles, Gaps::default());
        let id = tiles[0].id;

        set.current_mut().apply(LayoutOp::CyclePreset(id), &input);
        set.set_active(LayoutKind::MasterStack);
        set.forget(id);
        set.set_active(LayoutKind::ScrollingColumns);

        let mut out = LayoutOutput::default();
        set.current_mut().layout(&input, &mut out);
        let mut fresh = ScrollingColumns::default();
        let mut expected = LayoutOutput::default();
        fresh.layout(&input, &mut expected);
        assert_eq!(out.rects, expected.rects, "per-window state must be dropped");
    }
}

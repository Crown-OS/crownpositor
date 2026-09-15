//! dwm-style: a master column plus a stack.

use smithay::utils::{Logical, Rectangle};

use crate::{
    layout::{
        Direction, LayoutAlgorithm, LayoutInput, LayoutKind, LayoutOp, LayoutOutput, ResizeEdge,
        split_rows,
    },
    utils::id::WindowId,
};

const MIN_RATIO: f64 = 0.1;
const MAX_RATIO: f64 = 0.9;

#[derive(Debug)]
pub struct MasterStack {
    /// Fraction of the area's width given to the master column.
    master_ratio: f64,
    /// How many leading tiles share the master column.
    master_count: usize,
}

impl Default for MasterStack {
    fn default() -> Self {
        Self {
            master_ratio: 0.55,
            master_count: 1,
        }
    }
}

impl MasterStack {
    pub fn new(master_ratio: f64) -> Self {
        Self {
            master_ratio: master_ratio.clamp(MIN_RATIO, MAX_RATIO),
            ..Self::default()
        }
    }

    pub fn master_ratio(&self) -> f64 {
        self.master_ratio
    }

    fn set_ratio(&mut self, ratio: f64) -> bool {
        let clamped = ratio.clamp(MIN_RATIO, MAX_RATIO);
        let changed = (clamped - self.master_ratio).abs() > f64::EPSILON;
        self.master_ratio = clamped;
        changed
    }
}

impl LayoutAlgorithm for MasterStack {
    fn kind(&self) -> LayoutKind {
        LayoutKind::MasterStack
    }

    fn layout(&mut self, input: &LayoutInput<'_>, out: &mut LayoutOutput) {
        out.clear();

        let count = input.tiles.len();
        let masters = self.master_count.min(count);
        let stacked = count - masters;

        // With nothing in the stack the master column takes the whole area,
        // rather than leaving the right half empty.
        if stacked == 0 {
            out.rects
                .extend(split_rows(input.area, count, input.gaps.inner));
            return;
        }

        let columns = split_columns_by_ratio(input.area, self.master_ratio, input.gaps.inner);
        out.rects
            .extend(split_rows(columns.0, masters, input.gaps.inner));
        out.rects
            .extend(split_rows(columns.1, stacked, input.gaps.inner));
    }

    fn apply(&mut self, op: LayoutOp, input: &LayoutInput<'_>) -> bool {
        match op {
            LayoutOp::Grow(delta) => self.set_ratio(self.master_ratio + delta),
            LayoutOp::ResetSize(_) => self.set_ratio(Self::default().master_ratio),

            LayoutOp::PromoteDemote(id) => {
                let Some(index) = input.index_of(id) else {
                    return false;
                };
                // In the master area already -> demote it, otherwise promote.
                let target = if index < self.master_count {
                    self.master_count.saturating_sub(1)
                } else {
                    (self.master_count + 1).min(input.tiles.len())
                };
                let changed = target != self.master_count && target > 0;
                if changed {
                    self.master_count = target;
                }
                changed
            }

            LayoutOp::DragEdge { edge, delta, .. } => {
                let width = input.area.size.w.max(1) as f64;
                match edge {
                    ResizeEdge::Left => self.set_ratio(self.master_ratio - delta.x / width),
                    ResizeEdge::Right => self.set_ratio(self.master_ratio + delta.x / width),
                    // The stack splits evenly, so vertical drags have no knob.
                    ResizeEdge::Top | ResizeEdge::Bottom => false,
                }
            }

            LayoutOp::CyclePreset(_) => false,
        }
    }

    fn forget(&mut self, _id: WindowId) {}

    fn neighbour(
        &self,
        input: &LayoutInput<'_>,
        from: WindowId,
        dir: Direction,
    ) -> Option<WindowId> {
        let index = input.index_of(from)?;
        let masters = self.master_count.min(input.tiles.len());
        let in_master = index < masters;

        match dir {
            // Horizontal movement crosses the master/stack boundary.
            Direction::Right if in_master => input.tiles.get(masters).map(|tile| tile.id),
            Direction::Left if !in_master => input.tiles.get(masters - 1).map(|tile| tile.id),
            // Vertical movement stays in the column, which the workspace's
            // list-order fallback already gets right.
            _ => None,
        }
    }
}

fn split_columns_by_ratio(
    area: Rectangle<i32, Logical>,
    ratio: f64,
    gap: i32,
) -> (Rectangle<i32, Logical>, Rectangle<i32, Logical>) {
    let usable = (area.size.w - gap).max(2);
    let master_width = ((usable as f64 * ratio).round() as i32).clamp(1, usable - 1);

    let master = Rectangle::new(area.loc, (master_width, area.size.h).into());
    let stack = Rectangle::new(
        (area.loc.x + master_width + gap, area.loc.y).into(),
        (usable - master_width, area.size.h).into(),
    );
    (master, stack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Gaps, testing::*};

    const NO_GAPS: Gaps = Gaps { inner: 0, outer: 0 };

    fn run(layout: &mut MasterStack, input: &LayoutInput<'_>) -> Vec<Rectangle<i32, Logical>> {
        let mut out = LayoutOutput::default();
        layout.layout(input, &mut out);
        assert_eq!(out.rects.len(), input.tiles.len(), "one rect per tile");
        out.rects
    }

    #[test]
    fn a_lone_window_fills_the_area() {
        let tiles = tiles(1);
        let rects = run(
            &mut MasterStack::default(),
            &input(area(800, 600), &tiles, NO_GAPS),
        );
        assert_eq!(rects[0], area(800, 600));
    }

    #[test]
    fn the_master_column_gets_its_ratio() {
        let tiles = tiles(2);
        let rects = run(
            &mut MasterStack::new(0.5),
            &input(area(800, 600), &tiles, NO_GAPS),
        );
        assert_eq!(rects[0], Rectangle::new((0, 0).into(), (400, 600).into()));
        assert_eq!(rects[1], Rectangle::new((400, 0).into(), (400, 600).into()));
    }

    #[test]
    fn the_stack_splits_evenly_and_fills_the_column() {
        let tiles = tiles(4);
        let rects = run(
            &mut MasterStack::new(0.5),
            &input(area(800, 601), &tiles, NO_GAPS),
        );
        assert_covers_vertically(&rects[1..], area(800, 601), 0);
    }

    #[test]
    fn gaps_sit_between_tiles_not_around_them() {
        let tiles = tiles(3);
        let gaps = Gaps {
            inner: 10,
            outer: 0,
        };
        let rects = run(
            &mut MasterStack::new(0.5),
            &input(area(800, 600), &tiles, gaps),
        );

        assert_eq!(rects[0].loc.x, 0, "no gap at the outer edge");
        assert_eq!(rects[1].loc.x - (rects[0].loc.x + rects[0].size.w), 10);
        assert_eq!(rects[2].loc.y - (rects[1].loc.y + rects[1].size.h), 10);
        let last = rects[2];
        assert_eq!(last.loc.y + last.size.h, 600);
    }

    #[test]
    fn the_ratio_clamps_at_both_ends() {
        let tiles = tiles(2);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = MasterStack::default();

        while layout.apply(LayoutOp::Grow(0.2), &input) {}
        assert_eq!(layout.master_ratio(), MAX_RATIO);
        assert!(
            !layout.apply(LayoutOp::Grow(0.2), &input),
            "growing at the limit reports no change, so nothing is marked dirty"
        );

        while layout.apply(LayoutOp::Grow(-0.2), &input) {}
        assert_eq!(layout.master_ratio(), MIN_RATIO);
    }

    #[test]
    fn a_clamped_ratio_still_leaves_both_columns_visible() {
        let tiles = tiles(2);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = MasterStack::default();
        while layout.apply(LayoutOp::Grow(0.2), &input) {}

        let rects = run(&mut layout, &input);
        assert!(rects[0].size.w > 0 && rects[1].size.w > 0);
        assert_eq!(rects[0].size.w + rects[1].size.w, 800);
    }

    #[test]
    fn promote_moves_a_stack_window_into_master() {
        let tiles = tiles(3);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = MasterStack::default();

        assert!(layout.apply(LayoutOp::PromoteDemote(tiles[2].id), &input));
        let rects = run(&mut layout, &input);
        // Two masters now share the left column.
        assert_eq!(rects[0].loc.x, rects[1].loc.x);
        assert_ne!(rects[0].loc.x, rects[2].loc.x);
    }

    #[test]
    fn demote_never_empties_the_master_area() {
        let tiles = tiles(2);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = MasterStack::default();
        assert!(
            !layout.apply(LayoutOp::PromoteDemote(tiles[0].id), &input),
            "the last master cannot be demoted"
        );
    }

    #[test]
    fn horizontal_neighbours_cross_the_split() {
        let tiles = tiles(3);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let layout = MasterStack::default();

        assert_eq!(
            layout.neighbour(&input, tiles[0].id, Direction::Right),
            Some(tiles[1].id)
        );
        assert_eq!(
            layout.neighbour(&input, tiles[2].id, Direction::Left),
            Some(tiles[0].id)
        );
        assert_eq!(
            layout.neighbour(&input, tiles[0].id, Direction::Down),
            None,
            "vertical falls back to list order"
        );
    }
}

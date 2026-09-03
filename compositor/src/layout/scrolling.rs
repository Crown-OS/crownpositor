//! niri/PaperWM-style: an unbounded horizontal ribbon of columns.
//!
//! Columns are laid out at their running sum along x and the area's width is
//! ignored; `view_offset` is what brings the focused column on screen. Opening a
//! window therefore never resizes its neighbours.

use std::collections::HashMap;

use smithay::utils::{Logical, Point, Rectangle};

use crate::{
    layout::{
        Direction, LayoutAlgorithm, LayoutInput, LayoutKind, LayoutOp, LayoutOutput, ResizeEdge,
    },
    utils::id::WindowId,
};

const MIN_WIDTH: f64 = 0.1;
const MAX_WIDTH: f64 = 1.0;

#[derive(Debug)]
pub struct ScrollingColumns {
    /// Column width as a fraction of the area, per window. This is the
    /// per-window state that makes `forget` load-bearing.
    widths: HashMap<WindowId, f64>,
    presets: Vec<f64>,
    default_width: f64,
    /// Horizontal pan in logical pixels, published as `view_offset`.
    view_offset: f64,
}

impl Default for ScrollingColumns {
    fn default() -> Self {
        Self {
            widths: HashMap::new(),
            presets: vec![0.33333, 0.5, 0.66667],
            default_width: 0.5,
            view_offset: 0.0,
        }
    }
}

impl ScrollingColumns {
    pub fn new(default_width: f64) -> Self {
        Self {
            default_width: default_width.clamp(MIN_WIDTH, MAX_WIDTH),
            ..Self::default()
        }
    }

    pub fn view_offset(&self) -> f64 {
        self.view_offset
    }

    fn width_of(&self, id: WindowId) -> f64 {
        *self.widths.get(&id).unwrap_or(&self.default_width)
    }

    /// Column rects in ribbon coordinates, before the viewport offset.
    fn columns(&self, input: &LayoutInput<'_>) -> Vec<Rectangle<i32, Logical>> {
        let gap = input.gaps.inner;
        let full = input.area.size.w.max(1) as f64;

        let mut rects = Vec::with_capacity(input.tiles.len());
        let mut x = input.area.loc.x;

        for tile in input.tiles {
            let width = ((full * self.width_of(tile.id)).round() as i32).max(1);
            let size = tile.constrain((width, input.area.size.h).into());
            rects.push(Rectangle::new((x, input.area.loc.y).into(), size));
            x += size.w + gap;
        }

        rects
    }

    /// Pans the smallest distance that brings `index` fully into view.
    fn reveal_index(&mut self, index: usize, input: &LayoutInput<'_>) -> bool {
        let columns = self.columns(input);
        let Some(rect) = columns.get(index) else {
            return false;
        };

        let viewport_left = self.view_offset;
        let viewport_right = viewport_left + input.area.size.w as f64;
        let (left, right) = (rect.loc.x as f64, (rect.loc.x + rect.size.w) as f64);

        let target = if left < viewport_left {
            left
        } else if right > viewport_right {
            right - input.area.size.w as f64
        } else {
            return false;
        };

        let changed = (target - self.view_offset).abs() > f64::EPSILON;
        self.view_offset = target;
        changed
    }

    fn set_width(&mut self, id: WindowId, width: f64) -> bool {
        let clamped = width.clamp(MIN_WIDTH, MAX_WIDTH);
        let previous = self.width_of(id);
        self.widths.insert(id, clamped);
        (clamped - previous).abs() > f64::EPSILON
    }
}

impl LayoutAlgorithm for ScrollingColumns {
    fn kind(&self) -> LayoutKind {
        LayoutKind::ScrollingColumns
    }

    fn layout(&mut self, input: &LayoutInput<'_>, out: &mut LayoutOutput) {
        out.clear();
        out.rects.extend(self.columns(input));
        out.view_offset = Point::from((-self.view_offset, 0.0));
    }

    fn apply(&mut self, op: LayoutOp, input: &LayoutInput<'_>) -> bool {
        match op {
            LayoutOp::Grow(delta) => {
                let Some(id) = input.focused else {
                    return false;
                };
                self.set_width(id, self.width_of(id) + delta)
            }

            LayoutOp::ResetSize(id) => self.set_width(id, self.default_width),

            LayoutOp::CyclePreset(id) => {
                let current = self.width_of(id);
                // Next preset strictly wider than the current width, wrapping.
                let next = self
                    .presets
                    .iter()
                    .find(|preset| **preset > current + 0.01)
                    .or_else(|| self.presets.first())
                    .copied();
                next.is_some_and(|width| self.set_width(id, width))
            }

            LayoutOp::PromoteDemote(id) => {
                // A column is its own master, so this cycles to full width.
                let full = (self.width_of(id) - MAX_WIDTH).abs() < f64::EPSILON;
                self.set_width(id, if full { self.default_width } else { MAX_WIDTH })
            }

            LayoutOp::DragEdge { id, edge, delta } => {
                let full = input.area.size.w.max(1) as f64;
                match edge {
                    ResizeEdge::Right => self.set_width(id, self.width_of(id) + delta.x / full),
                    ResizeEdge::Left => self.set_width(id, self.width_of(id) - delta.x / full),
                    // Columns always span the full height.
                    ResizeEdge::Top | ResizeEdge::Bottom => false,
                }
            }
        }
    }

    fn forget(&mut self, id: WindowId) {
        self.widths.remove(&id);
    }

    fn neighbour(
        &self,
        input: &LayoutInput<'_>,
        from: WindowId,
        dir: Direction,
    ) -> Option<WindowId> {
        let index = input.index_of(from)?;
        match dir {
            Direction::Left => index.checked_sub(1).and_then(|i| input.tiles.get(i)),
            Direction::Right => input.tiles.get(index + 1),
            // One window per column, so there is nothing above or below.
            Direction::Up | Direction::Down => None,
        }
        .map(|tile| tile.id)
    }

    fn scroll(&mut self, delta: Point<f64, Logical>, input: &LayoutInput<'_>) -> bool {
        let columns = self.columns(input);
        let ribbon = columns
            .last()
            .map(|rect| (rect.loc.x + rect.size.w) as f64)
            .unwrap_or_default();

        let max = (ribbon - input.area.size.w as f64).max(0.0);
        let target = (self.view_offset + delta.x).clamp(0.0, max);
        let changed = (target - self.view_offset).abs() > f64::EPSILON;
        self.view_offset = target;
        changed
    }

    fn reveal(&mut self, id: WindowId, input: &LayoutInput<'_>) -> bool {
        let Some(index) = input.index_of(id) else {
            return false;
        };
        self.reveal_index(index, input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Gaps, testing::*};

    const NO_GAPS: Gaps = Gaps { inner: 0, outer: 0 };

    fn run(layout: &mut ScrollingColumns, input: &LayoutInput<'_>) -> LayoutOutput {
        let mut out = LayoutOutput::default();
        layout.layout(input, &mut out);
        assert_eq!(out.rects.len(), input.tiles.len(), "one rect per tile");
        out
    }

    #[test]
    fn columns_run_off_the_edge_instead_of_shrinking() {
        let tiles = tiles(4);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let out = run(&mut ScrollingColumns::new(0.5), &input);

        // Every column keeps its width; the ribbon is simply wider than the view.
        assert!(out.rects.iter().all(|rect| rect.size.w == 400));
        let last = out.rects.last().unwrap();
        assert!(last.loc.x + last.size.w > 800);
    }

    #[test]
    fn opening_a_window_does_not_move_the_others() {
        let many = tiles(3);
        let mut layout = ScrollingColumns::new(0.5);

        let two = run(&mut layout, &input(area(800, 600), &many[..2], NO_GAPS));
        let three = run(&mut layout, &input(area(800, 600), &many, NO_GAPS));
        assert_eq!(&three.rects[..2], &two.rects[..]);
    }

    #[test]
    fn columns_span_the_full_height() {
        let tiles = tiles(3);
        let out = run(
            &mut ScrollingColumns::default(),
            &input(area(800, 600), &tiles, NO_GAPS),
        );
        assert!(out.rects.iter().all(|rect| rect.size.h == 600));
    }

    #[test]
    fn reveal_pans_the_focused_column_fully_into_view() {
        let tiles = tiles(4);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = ScrollingColumns::new(0.5);

        assert!(layout.reveal(tiles[3].id, &input));
        let out = run(&mut layout, &input);

        let rect = out.rects[3];
        let left = rect.loc.x as f64 + out.view_offset.x;
        let right = left + rect.size.w as f64;
        assert!(
            left >= 0.0 && right <= 800.0,
            "column {left}..{right} not in view"
        );
    }

    #[test]
    fn revealing_an_already_visible_column_does_nothing() {
        let tiles = tiles(4);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = ScrollingColumns::new(0.5);
        assert!(!layout.reveal(tiles[0].id, &input));
        assert_eq!(layout.view_offset(), 0.0);
    }

    #[test]
    fn scrolling_clamps_to_the_ribbon() {
        let tiles = tiles(3);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = ScrollingColumns::new(0.5);

        layout.scroll((-500.0, 0.0).into(), &input);
        assert_eq!(layout.view_offset(), 0.0, "cannot scroll before the start");

        layout.scroll((5000.0, 0.0).into(), &input);
        // Three 400px columns = 1200 of ribbon, 800 of viewport.
        assert_eq!(layout.view_offset(), 400.0);
    }

    #[test]
    fn a_ribbon_narrower_than_the_view_does_not_scroll() {
        let tiles = tiles(1);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = ScrollingColumns::new(0.5);
        assert!(!layout.scroll((900.0, 0.0).into(), &input));
    }

    #[test]
    fn widths_are_per_window() {
        let tiles = tiles(2);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = ScrollingColumns::new(0.5);

        layout.apply(LayoutOp::PromoteDemote(tiles[0].id), &input);
        let out = run(&mut layout, &input);
        assert_eq!(out.rects[0].size.w, 800);
        assert_eq!(out.rects[1].size.w, 400, "the other column is untouched");
    }

    #[test]
    fn forget_drops_a_windows_width() {
        let tiles = tiles(2);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = ScrollingColumns::new(0.5);

        layout.apply(LayoutOp::PromoteDemote(tiles[0].id), &input);
        layout.forget(tiles[0].id);
        let out = run(&mut layout, &input);
        assert_eq!(out.rects[0].size.w, 400, "back to the default width");
    }

    #[test]
    fn min_size_widens_a_column() {
        let mut tiles = tiles(1);
        tiles[0].min_size = (700, 0).into();
        let out = run(
            &mut ScrollingColumns::new(0.5),
            &input(area(800, 600), &tiles, NO_GAPS),
        );
        assert_eq!(out.rects[0].size.w, 700);
    }

    #[test]
    fn cycling_presets_wraps() {
        let tiles = tiles(1);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let mut layout = ScrollingColumns::default();
        let id = tiles[0].id;

        let mut seen = Vec::new();
        for _ in 0..4 {
            layout.apply(LayoutOp::CyclePreset(id), &input);
            seen.push(layout.width_of(id));
        }
        assert_eq!(seen[0], 0.66667, "next preset above the 0.5 default");
        assert_eq!(seen[1], 0.33333, "wraps to the first");
        assert_eq!(seen[2], 0.5);
    }

    #[test]
    fn neighbours_run_along_the_ribbon() {
        let tiles = tiles(3);
        let input = input(area(800, 600), &tiles, NO_GAPS);
        let layout = ScrollingColumns::default();

        assert_eq!(
            layout.neighbour(&input, tiles[1].id, Direction::Right),
            Some(tiles[2].id)
        );
        assert_eq!(
            layout.neighbour(&input, tiles[1].id, Direction::Left),
            Some(tiles[0].id)
        );
        assert_eq!(layout.neighbour(&input, tiles[0].id, Direction::Left), None);
        assert_eq!(layout.neighbour(&input, tiles[0].id, Direction::Up), None);
    }
}

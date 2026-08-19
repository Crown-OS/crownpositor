//! Placement policy for windows the layout does not position.
//!
//! [`NoTiling`] is the algorithm a `Floating` workspace uses: every window keeps
//! whatever rect it already has. [`place`] is the separate question of where a
//! window that has never been placed should go.

use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::{
    layout::{LayoutAlgorithm, LayoutInput, LayoutKind, LayoutOp, LayoutOutput},
    utils::id::WindowId,
};

/// Offset between successive cascaded windows.
const CASCADE_STEP: i32 = 32;
/// How far a cascade walks before returning to the top left.
const CASCADE_LIMIT: i32 = 8;

#[derive(Debug, Default)]
pub struct NoTiling;

impl LayoutAlgorithm for NoTiling {
    fn kind(&self) -> LayoutKind {
        LayoutKind::Floating
    }

    /// A floating workspace still owes one rect per tile, so the caller's
    /// "exactly one rect per input" contract holds. The workspace overwrites
    /// these with each window's own `floating_rect`.
    fn layout(&mut self, input: &LayoutInput<'_>, out: &mut LayoutOutput) {
        out.clear();
        out.rects
            .extend(input.tiles.iter().map(|_| input.area));
    }

    fn apply(&mut self, _op: LayoutOp, _input: &LayoutInput<'_>) -> bool {
        false
    }

    fn forget(&mut self, _id: WindowId) {}
}

/// Where a newly floated window goes.
///
/// Centred on its parent when it has one — a dialog belongs over the window that
/// opened it — otherwise cascaded from the top left so a burst of windows does
/// not land in one stack.
pub fn place(
    area: Rectangle<i32, Logical>,
    size: Size<i32, Logical>,
    parent: Option<Rectangle<i32, Logical>>,
    cascade: usize,
) -> Rectangle<i32, Logical> {
    let size = Size::from((size.w.min(area.size.w), size.h.min(area.size.h)));

    let loc = match parent {
        Some(parent) => center_in(parent, size),
        None => {
            let step = (cascade % CASCADE_LIMIT as usize) as i32 * CASCADE_STEP;
            let centered = center_in(area, size);
            Point::from((centered.x.min(area.loc.x + step), area.loc.y + step))
        }
    };

    clamp_into(Rectangle::new(loc, size), area)
}

fn center_in(within: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
    Point::from((
        within.loc.x + (within.size.w - size.w) / 2,
        within.loc.y + (within.size.h - size.h) / 2,
    ))
}

/// Keeps a floating rect on screen. Only the location moves — resizing a window
/// because it drifted would be a surprising thing to do to a dialog.
pub fn clamp_into(
    rect: Rectangle<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let max_x = (area.loc.x + area.size.w - rect.size.w).max(area.loc.x);
    let max_y = (area.loc.y + area.size.h - rect.size.h).max(area.loc.y);

    Rectangle::new(
        (
            rect.loc.x.clamp(area.loc.x, max_x),
            rect.loc.y.clamp(area.loc.y, max_y),
        )
            .into(),
        rect.size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::testing::area;

    #[test]
    fn a_dialog_centers_on_its_parent() {
        let parent = Rectangle::new((100, 100).into(), (600, 400).into());
        let rect = place(area(1920, 1080), (200, 100).into(), Some(parent), 0);
        assert_eq!(rect.loc, (300, 250).into());
    }

    #[test]
    fn parentless_windows_cascade() {
        let first = place(area(1920, 1080), (400, 300).into(), None, 0);
        let second = place(area(1920, 1080), (400, 300).into(), None, 1);
        assert_ne!(first.loc, second.loc);
        assert_eq!(second.loc.y - first.loc.y, CASCADE_STEP);
    }

    #[test]
    fn the_cascade_wraps_instead_of_walking_off_screen() {
        let first = place(area(1920, 1080), (400, 300).into(), None, 0);
        let wrapped = place(area(1920, 1080), (400, 300).into(), None, 8);
        assert_eq!(first.loc, wrapped.loc);
    }

    #[test]
    fn placement_stays_inside_the_area() {
        let parent = Rectangle::new((1800, 1000).into(), (100, 60).into());
        let rect = place(area(1920, 1080), (600, 400).into(), Some(parent), 0);
        assert!(rect.loc.x >= 0 && rect.loc.y >= 0);
        assert!(rect.loc.x + rect.size.w <= 1920);
        assert!(rect.loc.y + rect.size.h <= 1080);
    }

    #[test]
    fn oversized_windows_are_capped_to_the_area() {
        let rect = place(area(800, 600), (2000, 2000).into(), None, 0);
        assert_eq!(rect.size, (800, 600).into());
    }

    #[test]
    fn clamping_moves_but_never_resizes() {
        let rect = Rectangle::new((-50, 900).into(), (400, 300).into());
        let clamped = clamp_into(rect, area(800, 600));
        assert_eq!(clamped.size, rect.size);
        assert_eq!(clamped.loc, (0, 300).into());
    }

    #[test]
    fn a_window_larger_than_the_area_pins_to_the_origin() {
        let rect = Rectangle::new((50, 50).into(), (1000, 800).into());
        assert_eq!(clamp_into(rect, area(800, 600)).loc, (0, 0).into());
    }
}

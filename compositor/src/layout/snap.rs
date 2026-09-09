//! Drag-to-edge tiling for floating windows.
//!
//! Pure geometry, like the rest of this module: given where the pointer is and
//! what the workspace area is, name the zone; given a zone, name its rect. What
//! it means to be "in" a zone — the preview, the window state, the rect the
//! window came from — belongs to the shell.

use smithay::utils::{Logical, Point, Rectangle};

use crate::layout::Gaps;

/// How close to an edge the pointer must be, in logical pixels.
const EDGE_REACH: f64 = 12.0;
/// Fraction of a side's length, measured from each end, that names a corner
/// rather than the middle of that side. Large enough to hit without aiming.
const CORNER_SPAN: f64 = 0.25;

/// What a snap is measured against.
///
/// Two rectangles, because the user aims at one and the window lands in the
/// other: a zone is found by shoving the cursor at the edge of the *screen*,
/// which is a thing you can feel, while the window itself lands in the usable
/// area so it never covers a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapBounds {
    /// The whole output, workspace-local.
    pub output: Rectangle<i32, Logical>,
    /// Exclusive zones and the outer gap already subtracted.
    pub area: Rectangle<i32, Logical>,
    pub gaps: Gaps,
}

impl SnapBounds {
    pub fn zone_at(self, pointer: Point<f64, Logical>) -> Option<SnapZone> {
        SnapZone::at(pointer, self.output)
    }

    pub fn rect(self, zone: SnapZone) -> Rectangle<i32, Logical> {
        zone.rect(self.area, self.gaps)
    }
}

/// Where a dragged window would land if it were released now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapZone {
    LeftHalf,
    RightHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// The top edge. Resolves to the ordinary maximized state, not a rect of
    /// its own, so unmaximizing a snapped window works the way it always has.
    Maximize,
}

impl SnapZone {
    /// The zone the pointer is currently in, if any.
    ///
    /// `bounds` is the whole output rather than the usable area: a panel's
    /// exclusive zone is invisible to the hand, and a user shoving the cursor
    /// into the top of the screen means to maximize whether or not a bar is
    /// sitting there.
    ///
    /// The vertical edges are tested first, so the very corner of the screen —
    /// where the pointer is near the left edge *and* the top — gives a quarter
    /// rather than a maximize.
    pub fn at(pointer: Point<f64, Logical>, bounds: Rectangle<i32, Logical>) -> Option<Self> {
        if bounds.is_empty() {
            return None;
        }

        let left = bounds.loc.x as f64;
        let top = bounds.loc.y as f64;
        let right = left + bounds.size.w as f64;
        let bottom = top + bounds.size.h as f64;

        // Outside the area entirely — the pointer is on another output, or in a
        // panel's exclusive zone.
        if pointer.x < left - EDGE_REACH
            || pointer.x > right + EDGE_REACH
            || pointer.y < top - EDGE_REACH
            || pointer.y > bottom + EDGE_REACH
        {
            return None;
        }

        let corner = bounds.size.h as f64 * CORNER_SPAN;
        let near_top = pointer.y <= top + corner;
        let near_bottom = pointer.y >= bottom - corner;

        if pointer.x <= left + EDGE_REACH {
            return Some(match (near_top, near_bottom) {
                (true, _) => Self::TopLeft,
                (_, true) => Self::BottomLeft,
                _ => Self::LeftHalf,
            });
        }

        if pointer.x >= right - EDGE_REACH {
            return Some(match (near_top, near_bottom) {
                (true, _) => Self::TopRight,
                (_, true) => Self::BottomRight,
                _ => Self::RightHalf,
            });
        }

        (pointer.y <= top + EDGE_REACH).then_some(Self::Maximize)
    }

    /// The rect this zone occupies inside `area`.
    ///
    /// Halves and quarters are separated by the inner gap and cover the area
    /// exactly, so two snapped windows line up with what the tiling layout
    /// would have produced.
    pub fn rect(self, area: Rectangle<i32, Logical>, gaps: Gaps) -> Rectangle<i32, Logical> {
        let (left, right) = split(area.size.w, gaps.inner);
        let (top, bottom) = split(area.size.h, gaps.inner);

        let x_start = area.loc.x;
        let x_mid = area.loc.x + left + gaps.inner;
        let y_start = area.loc.y;
        let y_mid = area.loc.y + top + gaps.inner;

        let rect = |x, y, w, h| Rectangle::new((x, y).into(), (w, h).into());

        match self {
            Self::Maximize => area,
            Self::LeftHalf => rect(x_start, y_start, left, area.size.h),
            Self::RightHalf => rect(x_mid, y_start, right, area.size.h),
            Self::TopLeft => rect(x_start, y_start, left, top),
            Self::TopRight => rect(x_mid, y_start, right, top),
            Self::BottomLeft => rect(x_start, y_mid, left, bottom),
            Self::BottomRight => rect(x_mid, y_mid, right, bottom),
        }
    }
}

/// Two extents separated by `gap`, together covering `extent` exactly. The
/// leading half takes the odd pixel.
fn split(extent: i32, gap: i32) -> (i32, i32) {
    let usable = (extent - gap).max(2);
    let leading = usable - usable / 2;
    (leading, usable / 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::testing::area;

    const GAPS: Gaps = Gaps { inner: 8, outer: 8 };
    const NO_GAPS: Gaps = Gaps { inner: 0, outer: 0 };

    fn at(x: f64, y: f64) -> Option<SnapZone> {
        SnapZone::at((x, y).into(), area(1000, 800))
    }

    #[test]
    fn the_side_edges_give_halves() {
        assert_eq!(at(2.0, 400.0), Some(SnapZone::LeftHalf));
        assert_eq!(at(998.0, 400.0), Some(SnapZone::RightHalf));
    }

    #[test]
    fn the_top_edge_maximizes() {
        assert_eq!(at(500.0, 1.0), Some(SnapZone::Maximize));
    }

    #[test]
    fn the_bottom_edge_does_nothing() {
        assert_eq!(at(500.0, 799.0), None);
    }

    #[test]
    fn the_middle_is_not_a_zone() {
        assert_eq!(at(500.0, 400.0), None);
    }

    /// The corners belong to the vertical edges, so the very top left is a
    /// quarter and not a maximize.
    #[test]
    fn corners_beat_the_top_edge() {
        assert_eq!(at(1.0, 1.0), Some(SnapZone::TopLeft));
        assert_eq!(at(999.0, 1.0), Some(SnapZone::TopRight));
        assert_eq!(at(1.0, 799.0), Some(SnapZone::BottomLeft));
        assert_eq!(at(999.0, 799.0), Some(SnapZone::BottomRight));
    }

    #[test]
    fn a_pointer_far_outside_the_area_snaps_to_nothing() {
        assert_eq!(SnapZone::at((-200.0, 400.0).into(), area(1000, 800)), None);
    }

    #[test]
    fn an_empty_area_has_no_zones() {
        assert_eq!(SnapZone::at((0.0, 0.0).into(), area(0, 0)), None);
    }

    #[test]
    fn halves_cover_the_area_with_one_gap_between_them() {
        let (left, right) = (
            SnapZone::LeftHalf.rect(area(1000, 800), GAPS),
            SnapZone::RightHalf.rect(area(1000, 800), GAPS),
        );

        assert_eq!(left.loc.x, 0);
        assert_eq!(right.loc.x - (left.loc.x + left.size.w), GAPS.inner);
        assert_eq!(right.loc.x + right.size.w, 1000);
        assert_eq!(left.size.h, 800, "a half spans the full height");
        assert_eq!(right.size.h, 800);
    }

    /// An odd width must not leave a one-pixel stripe of background.
    #[test]
    fn an_odd_extent_still_covers_exactly() {
        let left = SnapZone::LeftHalf.rect(area(999, 800), NO_GAPS);
        let right = SnapZone::RightHalf.rect(area(999, 800), NO_GAPS);
        assert_eq!(left.size.w + right.size.w, 999);
        assert_eq!(right.loc.x, left.size.w);
    }

    #[test]
    fn quarters_tile_the_area() {
        let zones = [
            SnapZone::TopLeft,
            SnapZone::TopRight,
            SnapZone::BottomLeft,
            SnapZone::BottomRight,
        ];
        let rects: Vec<_> = zones.map(|zone| zone.rect(area(1000, 800), NO_GAPS)).into();

        let covered: i32 = rects.iter().map(|rect| rect.size.w * rect.size.h).sum();
        assert_eq!(covered, 1000 * 800, "the four quarters cover the area");
        assert_eq!(rects[0].loc, (0, 0).into());
        assert_eq!(rects[3].loc, (500, 400).into());
    }

    #[test]
    fn maximize_takes_the_whole_area() {
        assert_eq!(
            SnapZone::Maximize.rect(area(1000, 800), GAPS),
            area(1000, 800)
        );
    }

    /// A bar across the top reserves its strip, and the usable area starts
    /// below it. The cursor still has to snap at the *screen* edge: nobody can
    /// feel where an exclusive zone begins.
    #[test]
    fn the_screen_edge_snaps_even_with_a_panel_in_the_way() {
        let bounds = SnapBounds {
            output: area(1000, 800),
            area: Rectangle::new((8, 48).into(), (984, 744).into()),
            gaps: GAPS,
        };

        assert_eq!(
            bounds.zone_at((500.0, 1.0).into()),
            Some(SnapZone::Maximize)
        );
        assert_eq!(
            bounds.zone_at((1.0, 400.0).into()),
            Some(SnapZone::LeftHalf)
        );
    }

    /// ...and what it lands in is still the usable area, so a maximized window
    /// does not cover the bar it snapped past.
    #[test]
    fn a_snap_lands_below_the_panel_it_snapped_past() {
        let usable = Rectangle::new((8, 48).into(), (984, 744).into());
        let bounds = SnapBounds {
            output: area(1000, 800),
            area: usable,
            gaps: GAPS,
        };

        assert_eq!(bounds.rect(SnapZone::Maximize), usable);
        assert_eq!(bounds.rect(SnapZone::LeftHalf).loc.y, usable.loc.y);
    }
}

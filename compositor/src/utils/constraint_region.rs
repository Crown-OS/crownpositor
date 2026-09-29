//! Geometry for `zwp_confined_pointer_v1`: where a confined pointer may go.

use smithay::{
    utils::{Logical, Point, Size},
    wayland::compositor::RegionAttributes,
};

/// Whether a surface-local point is inside the constraint.
///
/// A constraint without a region covers the whole surface, and one with a
/// region is still clipped to the surface: the protocol intersects the two.
pub fn allows(
    region: Option<&RegionAttributes>,
    surface_size: Size<i32, Logical>,
    point: Point<f64, Logical>,
) -> bool {
    let on_surface = point.x >= 0.0
        && point.y >= 0.0
        && point.x < f64::from(surface_size.w)
        && point.y < f64::from(surface_size.h);
    on_surface && region.is_none_or(|region| region.contains(point.to_i32_floor()))
}

/// Where a confined pointer heading from `from` to `to` ends up.
///
/// A move that would leave the region slides along the edge it hit instead of
/// stopping dead, which is what makes a confined pointer feel like it is
/// against a wall rather than stuck to it.
pub fn confine(
    from: Point<f64, Logical>,
    to: Point<f64, Logical>,
    allowed: impl Fn(Point<f64, Logical>) -> bool,
) -> Point<f64, Logical> {
    [to, Point::from((to.x, from.y)), Point::from((from.x, to.y))]
        .into_iter()
        .find(|candidate| allowed(*candidate))
        .unwrap_or(from)
}

#[cfg(test)]
mod tests {
    use smithay::{utils::Rectangle, wayland::compositor::RectangleKind};

    use super::*;

    fn region(rects: &[(RectangleKind, (i32, i32, i32, i32))]) -> RegionAttributes {
        RegionAttributes {
            rects: rects
                .iter()
                .map(|(kind, (x, y, w, h))| {
                    (*kind, Rectangle::new((*x, *y).into(), (*w, *h).into()))
                })
                .collect(),
        }
    }

    fn surface() -> Size<i32, Logical> {
        Size::from((100, 100))
    }

    #[test]
    fn no_region_means_the_whole_surface() {
        assert!(allows(None, surface(), (0.0, 0.0).into()));
        assert!(allows(None, surface(), (99.5, 99.5).into()));
        assert!(
            !allows(None, surface(), (100.0, 50.0).into()),
            "the far edge is outside"
        );
        assert!(!allows(None, surface(), (-0.1, 50.0).into()));
    }

    #[test]
    fn a_region_is_clipped_to_the_surface() {
        let oversized = region(&[(RectangleKind::Add, (-50, -50, 400, 400))]);
        assert!(!allows(Some(&oversized), surface(), (150.0, 10.0).into()));
        assert!(allows(Some(&oversized), surface(), (10.0, 10.0).into()));
    }

    #[test]
    fn subtracted_rectangles_are_holes() {
        let ring = region(&[
            (RectangleKind::Add, (0, 0, 100, 100)),
            (RectangleKind::Subtract, (25, 25, 50, 50)),
        ]);
        assert!(allows(Some(&ring), surface(), (10.0, 10.0).into()));
        assert!(!allows(Some(&ring), surface(), (50.0, 50.0).into()));
    }

    #[test]
    fn a_move_inside_is_taken_whole() {
        let allowed = |p: Point<f64, Logical>| allows(None, surface(), p);
        let to = Point::from((60.0, 70.0));
        assert_eq!(confine((50.0, 50.0).into(), to, allowed), to);
    }

    #[test]
    fn a_move_across_an_edge_slides_along_it() {
        let allowed = |p: Point<f64, Logical>| allows(None, surface(), p);
        let slid = confine((50.0, 50.0).into(), (150.0, 60.0).into(), allowed);
        assert_eq!(slid, Point::from((50.0, 60.0)));
    }

    #[test]
    fn a_move_into_a_corner_stays_put() {
        let l_shape = region(&[
            (RectangleKind::Add, (0, 0, 50, 100)),
            (RectangleKind::Add, (0, 50, 100, 50)),
        ]);
        let allowed = |p: Point<f64, Logical>| allows(Some(&l_shape), surface(), p);
        let from = Point::from((49.0, 0.0));
        assert_eq!(confine(from, (55.0, -5.0).into(), allowed), from);
        assert_eq!(
            confine((40.0, 40.0).into(), (60.0, 20.0).into(), allowed),
            Point::from((40.0, 20.0)),
            "blocked sideways, still free to move up"
        );
    }
}

//! Where a rounded shape is certainly opaque.
//!
//! A rounded rectangle is one rectangle inside its corners' reach along one
//! axis and two more along the other, and only those are rectangles: the arcs
//! are left out rather than approximated, because claiming a pixel that is
//! not there would leave a hole in whatever is under it. The damage tracker
//! skips everything these pieces cover, which is what keeps the wallpaper,
//! the glass behind a frame and every window below from being drawn — and
//! blurred — under an opaque window.

use smithay::{
    backend::renderer::utils::OpaqueRegions,
    utils::{Physical, Point, Rectangle},
};

use crate::utils::{region, squircle};

type Rect = Rectangle<i32, Physical>;

/// A pixel of slack around a shape, so the antialiased edge is never counted
/// as opaque.
pub const MARGIN: i32 = 1;

/// The disjoint rectangles of `shape` that lie inside the reach of corners of
/// `radii`, in the space `shape` is in.
pub fn bands(shape: Rect, radii: [f32; 4]) -> impl Iterator<Item = Rect> {
    let radius = radii.into_iter().fold(0.0_f32, f32::max);
    let reach = squircle::reach(radius).ceil() as i32 + MARGIN;
    let (left, top) = (shape.loc.x, shape.loc.y);
    let (right, bottom) = (left + shape.size.w, top + shape.size.h);
    [
        piece(left + reach, top + MARGIN, right - reach, bottom - MARGIN),
        piece(left + MARGIN, top + reach, left + reach, bottom - reach),
        piece(right - reach, top + reach, right - MARGIN, bottom - reach),
    ]
    .into_iter()
    .flatten()
}

fn piece(left: i32, top: i32, right: i32, bottom: i32) -> Option<Rect> {
    (right > left && bottom > top).then(|| Rect::from_extremities((left, top), (right, bottom)))
}

/// `regions`, relative to an element at `location`, kept to where the
/// output-local `shape` it is cut to is certainly opaque.
pub fn within(
    regions: &OpaqueRegions<i32, Physical>,
    shape: Rect,
    radii: [f32; 4],
    location: Point<i32, Physical>,
) -> OpaqueRegions<i32, Physical> {
    bands(shape, radii)
        .flat_map(|band| {
            let band = Rectangle::new(band.loc - location, band.size);
            regions
                .iter()
                .filter_map(move |region| region.intersection(band))
        })
        .collect()
}

/// The parts of `shape` its corners of `radius` cut away, plus the margin.
pub fn corners(shape: Rect, radius: f32) -> Vec<Rect> {
    region::subtract([shape], bands(shape, [radius; 4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect::new((x, y).into(), (w, h).into())
    }

    fn covered(rects: impl IntoIterator<Item = Rect>, point: (i32, i32)) -> bool {
        rects.into_iter().any(|rect| rect.contains(point))
    }

    #[test]
    fn a_square_shape_keeps_everything_but_its_edge() {
        let bands: Vec<_> = bands(rect(0, 0, 100, 50), [0.0; 4]).collect();
        assert_eq!(bands, vec![rect(1, 1, 98, 48)]);
    }

    /// A squircle corner of radius 20 starts curving 30 pixels in, so nothing
    /// within that reach of a corner is claimed, and the three pieces never
    /// overlap.
    #[test]
    fn a_rounded_shape_leaves_its_corners_out_once() {
        let shape = rect(0, 0, 100, 100);
        let bands: Vec<_> = bands(shape, [20.0, 0.0, 0.0, 0.0]).collect();
        assert_eq!(
            bands,
            vec![
                rect(31, 1, 38, 98),
                rect(1, 31, 30, 38),
                rect(69, 31, 30, 38)
            ]
        );
        assert!(!covered(bands.iter().copied(), (4, 4)));
        assert!(!covered(bands.iter().copied(), (25, 2)));
        assert!(covered(bands.iter().copied(), (50, 50)));
        let area: i32 = bands.iter().map(|band| band.size.w * band.size.h).sum();
        assert_eq!(area, 38 * 98 + 2 * 30 * 38);
    }

    #[test]
    fn a_shape_smaller_than_its_curve_is_opaque_nowhere() {
        assert_eq!(bands(rect(0, 0, 4, 4), [2.0; 4]).count(), 0);
    }

    #[test]
    fn regions_are_kept_to_the_bands_in_the_elements_own_space() {
        let regions = OpaqueRegions::from_slice(&[rect(0, 0, 100, 100)]);
        let kept = within(&regions, rect(10, 20, 100, 100), [0.0; 4], (10, 20).into());
        assert_eq!(
            kept.iter().copied().collect::<Vec<_>>(),
            vec![rect(1, 1, 98, 98)]
        );
    }

    #[test]
    fn the_corners_are_what_the_bands_leave_out() {
        let shape = rect(0, 0, 100, 100);
        let corners = corners(shape, 20.0);
        for point in [(0, 0), (4, 4), (25, 2), (99, 99), (2, 97)] {
            assert!(covered(corners.iter().copied(), point), "{point:?}");
        }
        assert!(!covered(corners.iter().copied(), (50, 50)));
    }
}

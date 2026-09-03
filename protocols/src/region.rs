//! Turning a `wl_region` into rectangles a renderer can draw.
//!
//! A `wl_region` is a *set*, built by adding and subtracting overlapping
//! rectangles in order. Nothing about that list is drawable: rects overlap,
//! later ones punch holes in earlier ones, and drawing it verbatim would
//! composite the same pixel several times — visible immediately once the thing
//! being drawn is translucent, which is exactly the case for a blur backdrop.
//!
//! [`region_rects`] normalises the set into a list of disjoint rectangles that
//! covers it exactly, so callers can loop over them without thinking about
//! order or overlap.

use std::collections::BTreeSet;

use smithay::{
    utils::{Logical, Rectangle},
    wayland::compositor::{RectangleKind, RegionAttributes},
};

/// Decomposes `region` into non-overlapping rectangles, in `out`.
///
/// The result covers exactly the same points as the region, contains no
/// duplicates or overlaps, and is empty when the region is.
///
/// The method is a scanline sweep: every rect edge contributes a horizontal
/// cut, and inside each resulting band the region is one-dimensional — a
/// sorted list of X spans that adds and subtracts collapse into trivially.
/// Bands whose spans came out identical are then merged back vertically, so
/// the common cases (one rect; a rect with a hole) come back as few rects
/// rather than one per cut line.
pub fn region_rects(region: &RegionAttributes, out: &mut Vec<Rectangle<i32, Logical>>) {
    out.clear();

    // Degenerate rects contribute no area but would still cut bands, so they
    // are dropped up front rather than re-checked in every band.
    let contributes = |rect: &Rectangle<i32, Logical>| rect.size.w > 0 && rect.size.h > 0;

    // `BTreeSet` because the sweep needs these sorted and deduplicated, and a
    // region rarely has enough rects for anything cleverer to pay off.
    let cuts: BTreeSet<i32> = region
        .rects
        .iter()
        .filter(|(_, rect)| contributes(rect))
        .flat_map(|(_, rect)| [rect.loc.y, rect.loc.y + rect.size.h])
        .collect();

    let mut cuts = cuts.into_iter();
    // Fewer than two cut lines means no band, which means no area.
    let Some(mut lo) = cuts.next() else {
        return;
    };

    // The band currently being accumulated, kept open so vertically identical
    // bands merge into one rect instead of stacking up.
    let mut open: Option<(i32, Vec<(i32, i32)>)> = None;
    let mut spans: Vec<(i32, i32)> = Vec::new();

    for hi in cuts {
        spans.clear();
        for (kind, rect) in &region.rects {
            // Rects that do not cross this band cannot affect it — including
            // the ones that were dropped from the cut set.
            if !contributes(rect) || hi <= rect.loc.y || rect.loc.y + rect.size.h <= lo {
                continue;
            }

            let (x1, x2) = (rect.loc.x, rect.loc.x + rect.size.w);
            match kind {
                RectangleKind::Add => add_span(&mut spans, x1, x2),
                RectangleKind::Subtract => subtract_span(&mut spans, x1, x2),
            }
        }

        match &mut open {
            // Same shape as the band above: leave it open and let it grow.
            Some((_, previous)) if *previous == spans => {}
            Some((top, previous)) => {
                emit(previous, *top, lo, out);
                previous.clone_from(&spans);
                *top = lo;
            }
            None => open = Some((lo, spans.clone())),
        }

        lo = hi;
    }

    if let Some((top, previous)) = &open {
        emit(previous, *top, lo, out);
    }
}

fn emit(spans: &[(i32, i32)], top: i32, bottom: i32, out: &mut Vec<Rectangle<i32, Logical>>) {
    out.extend(
        spans
            .iter()
            .map(|(x1, x2)| Rectangle::from_extremities((*x1, top), (*x2, bottom))),
    );
}

/// Unions `[x1, x2)` into a sorted list of disjoint spans.
///
/// Spans that merely *touch* are absorbed too: leaving `[0,5)` and `[5,10)`
/// separate would be correct as a set but would hand the renderer a seam that
/// blending can show through.
fn add_span(spans: &mut Vec<(i32, i32)>, mut x1: i32, mut x2: i32) {
    if x1 >= x2 {
        return;
    }

    let mut at = 0;
    while at < spans.len() && spans[at].1 < x1 {
        at += 1;
    }
    while at < spans.len() && spans[at].0 <= x2 {
        let (start, end) = spans.remove(at);
        x1 = x1.min(start);
        x2 = x2.max(end);
    }
    spans.insert(at, (x1, x2));
}

/// Removes `[x1, x2)` from a sorted list of disjoint spans, splitting the
/// spans it lands in the middle of.
fn subtract_span(spans: &mut Vec<(i32, i32)>, x1: i32, x2: i32) {
    if x1 >= x2 {
        return;
    }

    let mut at = 0;
    while at < spans.len() {
        let (start, end) = spans[at];
        // Touching is not overlapping here: `[0,5)` survives subtracting
        // `[5,10)` whole.
        if end <= x1 || x2 <= start {
            at += 1;
            continue;
        }

        spans.remove(at);
        if start < x1 {
            spans.insert(at, (start, x1));
            at += 1;
        }
        if x2 < end {
            spans.insert(at, (x2, end));
            at += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

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

    fn rects_of(region: &RegionAttributes) -> Vec<Rectangle<i32, Logical>> {
        let mut out = Vec::new();
        region_rects(region, &mut out);
        out
    }

    /// The region as a point set, by applying the rects in order — the
    /// definition [`region_rects`] has to agree with.
    fn points_by_definition(region: &RegionAttributes) -> HashSet<(i32, i32)> {
        let mut points = HashSet::new();
        for (kind, rect) in &region.rects {
            for y in rect.loc.y..rect.loc.y + rect.size.h {
                for x in rect.loc.x..rect.loc.x + rect.size.w {
                    match kind {
                        RectangleKind::Add => {
                            points.insert((x, y));
                        }
                        RectangleKind::Subtract => {
                            points.remove(&(x, y));
                        }
                    }
                }
            }
        }
        points
    }

    /// The same set, from the decomposition — panicking if any two output
    /// rects overlap, which is the property the renderer depends on.
    fn points_by_decomposition(region: &RegionAttributes) -> HashSet<(i32, i32)> {
        let mut points = HashSet::new();
        for rect in rects_of(region) {
            assert!(
                rect.size.w > 0 && rect.size.h > 0,
                "emitted an empty rect: {rect:?}"
            );
            for y in rect.loc.y..rect.loc.y + rect.size.h {
                for x in rect.loc.x..rect.loc.x + rect.size.w {
                    assert!(points.insert((x, y)), "rects overlap at {x},{y}");
                }
            }
        }
        points
    }

    fn assert_exact(region: &RegionAttributes) {
        assert_eq!(
            points_by_decomposition(region),
            points_by_definition(region)
        );
    }

    #[test]
    fn empty_region_yields_nothing() {
        assert!(rects_of(&region(&[])).is_empty());
    }

    #[test]
    fn degenerate_rects_yield_nothing() {
        let region = region(&[
            (RectangleKind::Add, (0, 0, 0, 10)),
            (RectangleKind::Add, (0, 0, 10, 0)),
        ]);
        assert!(rects_of(&region).is_empty());
    }

    #[test]
    fn a_single_rect_survives_whole() {
        let region = region(&[(RectangleKind::Add, (3, 4, 10, 20))]);
        assert_eq!(
            rects_of(&region),
            vec![Rectangle::new((3, 4).into(), (10, 20).into())]
        );
    }

    #[test]
    fn stacked_identical_rects_do_not_multiply() {
        // The naive answer is two overlapping rects; the right one is one.
        let region = region(&[
            (RectangleKind::Add, (0, 0, 10, 10)),
            (RectangleKind::Add, (0, 0, 10, 10)),
        ]);
        assert_eq!(
            rects_of(&region),
            vec![Rectangle::new((0, 0).into(), (10, 10).into())]
        );
    }

    #[test]
    fn touching_rects_merge_into_one() {
        // A seam here would show as a line through a translucent backdrop.
        let region = region(&[
            (RectangleKind::Add, (0, 0, 10, 10)),
            (RectangleKind::Add, (10, 0, 10, 10)),
        ]);
        assert_eq!(
            rects_of(&region),
            vec![Rectangle::new((0, 0).into(), (20, 10).into())]
        );
    }

    #[test]
    fn overlapping_rects_are_split_disjointly() {
        assert_exact(&region(&[
            (RectangleKind::Add, (0, 0, 10, 10)),
            (RectangleKind::Add, (5, 5, 10, 10)),
        ]));
    }

    #[test]
    fn a_hole_becomes_a_ring() {
        // The case the old bounding-box approximation got wrong outright.
        let region = region(&[
            (RectangleKind::Add, (0, 0, 30, 30)),
            (RectangleKind::Subtract, (10, 10, 10, 10)),
        ]);
        assert_exact(&region);
        // Four bands' worth of rects, not thirty.
        assert_eq!(rects_of(&region).len(), 4);
    }

    #[test]
    fn subtracting_everything_leaves_nothing() {
        let region = region(&[
            (RectangleKind::Add, (0, 0, 10, 10)),
            (RectangleKind::Subtract, (-5, -5, 30, 30)),
        ]);
        assert!(rects_of(&region).is_empty());
    }

    #[test]
    fn adding_back_over_a_hole_fills_it() {
        // Order matters: the last rect wins where they overlap.
        let region = region(&[
            (RectangleKind::Add, (0, 0, 30, 30)),
            (RectangleKind::Subtract, (10, 10, 10, 10)),
            (RectangleKind::Add, (10, 10, 10, 10)),
        ]);
        assert_eq!(
            rects_of(&region),
            vec![Rectangle::new((0, 0).into(), (30, 30).into())]
        );
    }

    #[test]
    fn negative_coordinates_are_handled() {
        assert_exact(&region(&[
            (RectangleKind::Add, (-20, -20, 30, 30)),
            (RectangleKind::Subtract, (-10, -10, 5, 40)),
        ]));
    }

    #[test]
    fn matches_the_definition_on_a_pile_of_rects() {
        // A deterministic pseudo-random pile: enough overlap, nesting and
        // partial subtraction to exercise every branch of the sweep.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |bound: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as i32
        };

        for _ in 0..64 {
            let mut rects = Vec::new();
            for index in 0..8 {
                let kind = if index % 3 == 2 {
                    RectangleKind::Subtract
                } else {
                    RectangleKind::Add
                };
                let (x, y) = (next(20) - 10, next(20) - 10);
                let (w, h) = (next(12), next(12));
                rects.push((kind, (x, y, w, h)));
            }
            assert_exact(&region(&rects));
        }
    }
}

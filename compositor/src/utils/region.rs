//! Rectangle-set subtraction.
//!
//! smithay's `Rectangle::subtract_rects_many` scans its list forwards and
//! `swap_remove`s every hit, which pulls the fragments it has just appended
//! back into the part still to be scanned. Each of them spends one of the
//! scan's fixed number of checks, so an original rectangle further along is
//! never looked at and comes back whole, overlap and all. Scanning backwards
//! means whatever the swap pulls in has already been checked.

use smithay::utils::{Physical, Rectangle};

type Rect = Rectangle<i32, Physical>;

/// `rects` with every part that any of `others` covers taken out.
pub fn subtract(
    rects: impl IntoIterator<Item = Rect>,
    others: impl IntoIterator<Item = Rect>,
) -> Vec<Rect> {
    let mut remaining: Vec<Rect> = rects.into_iter().collect();
    for other in others {
        for index in (0..remaining.len()).rev() {
            let Some(overlap) = remaining[index].intersection(other) else {
                continue;
            };
            let rect = remaining.swap_remove(index);
            remaining.extend(around(rect, overlap));
        }
    }
    remaining
}

/// Merging two rectangles is worth it while their bounding box wastes at most
/// this fraction over the area they really cover.
const MERGE_SLACK: f64 = 1.25;
/// Past this many pieces, per-piece overhead outweighs the pixels saved.
const MAX_PIECES: usize = 12;

/// Disjoint rectangles covering the union of `rects` for drawing over: pairs
/// whose bounding box wastes little are merged, overlaps are cut away so no
/// pixel is covered twice, and a set that stays fragmented falls back to one
/// bounding box.
pub fn coalesce(rects: impl IntoIterator<Item = Rect>) -> Vec<Rect> {
    let mut merged: Vec<Rect> = rects.into_iter().filter(|rect| !rect.is_empty()).collect();
    while let Some((a, b)) = cheapest_merge(&merged) {
        let union = merged[a].merge(merged[b]);
        merged.swap_remove(b);
        merged[a] = union;
    }

    let mut disjoint: Vec<Rect> = Vec::with_capacity(merged.len());
    for rect in merged {
        let pieces = subtract([rect], disjoint.iter().copied());
        disjoint.extend(pieces);
    }
    if disjoint.len() > MAX_PIECES {
        return disjoint
            .into_iter()
            .reduce(Rect::merge)
            .into_iter()
            .collect();
    }
    disjoint
}

/// The pair whose merge wastes least, if any wastes little enough. `a < b`.
fn cheapest_merge(rects: &[Rect]) -> Option<(usize, usize)> {
    let area = |rect: Rect| rect.size.w as f64 * rect.size.h as f64;
    let mut best: Option<(f64, usize, usize)> = None;
    for a in 0..rects.len() {
        for b in a + 1..rects.len() {
            let overlap = rects[a].intersection(rects[b]).map_or(0.0, area);
            let covered = area(rects[a]) + area(rects[b]) - overlap;
            let ratio = area(rects[a].merge(rects[b])) / covered.max(1.0);
            if ratio <= MERGE_SLACK && best.is_none_or(|(cost, ..)| ratio < cost) {
                best = Some((ratio, a, b));
            }
        }
    }
    best.map(|(_, a, b)| (a, b))
}

/// Where `rects` and `others` overlap. Pieces may overlap each other when
/// `others` do.
pub fn intersect(rects: &[Rect], others: impl IntoIterator<Item = Rect>) -> Vec<Rect> {
    others
        .into_iter()
        .flat_map(|other| {
            rects
                .iter()
                .filter_map(move |rect| rect.intersection(other))
        })
        .collect()
}

/// The up to four pieces of `rect` that lie outside `overlap`, which it
/// contains: full-width bands above and below, and the two sides between.
fn around(rect: Rect, overlap: Rect) -> impl Iterator<Item = Rect> {
    let end = rect.loc + rect.size.to_point();
    let overlap_end = overlap.loc + overlap.size.to_point();
    [
        Rect::from_extremities(rect.loc, (end.x, overlap.loc.y)),
        Rect::from_extremities((rect.loc.x, overlap_end.y), end),
        Rect::from_extremities((rect.loc.x, overlap.loc.y), (overlap.loc.x, overlap_end.y)),
        Rect::from_extremities((overlap_end.x, overlap.loc.y), (end.x, overlap_end.y)),
    ]
    .into_iter()
    .filter(|piece| !piece.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect::new((x, y).into(), (w, h).into())
    }

    fn points(rects: &[Rect]) -> HashSet<(i32, i32)> {
        rects
            .iter()
            .flat_map(|rect| {
                (rect.loc.y..rect.loc.y + rect.size.h)
                    .flat_map(move |y| (rect.loc.x..rect.loc.x + rect.size.w).map(move |x| (x, y)))
            })
            .collect()
    }

    /// The case smithay gets wrong: two hits before an untouched third, and
    /// the third lies wholly inside what is being subtracted.
    #[test]
    fn a_rect_after_two_hits_is_still_subtracted() {
        let other = rect(10, 0, 10, 100);
        let left = subtract(
            [rect(0, 0, 30, 10), rect(12, 50, 5, 5), rect(0, 20, 30, 10)],
            [other],
        );
        assert!(!left.iter().any(|piece| piece.overlaps(other)), "{left:?}");
    }

    #[test]
    fn coalescing_covers_the_union_exactly_once() {
        let rects = [
            rect(0, 0, 40, 40),
            rect(30, 30, 40, 40),
            rect(200, 0, 10, 10),
            rect(5, 5, 10, 10),
        ];
        let coalesced = coalesce(rects);
        let covered: usize = coalesced
            .iter()
            .map(|r| (r.size.w * r.size.h) as usize)
            .sum();
        assert_eq!(covered, points(&coalesced).len(), "pieces overlap");
        assert!(points(&rects).is_subset(&points(&coalesced)));
    }

    #[test]
    fn distant_rectangles_stay_apart() {
        let coalesced = coalesce([rect(0, 0, 10, 10), rect(500, 500, 10, 10)]);
        assert_eq!(coalesced.len(), 2);
    }

    #[test]
    fn a_ring_is_not_filled_in() {
        // What a lower piece of glass is left with around an opaque one above.
        let ring = [
            rect(0, 0, 300, 20),
            rect(0, 280, 300, 20),
            rect(0, 20, 20, 260),
            rect(280, 20, 20, 260),
        ];
        let covered: usize = coalesce(ring)
            .iter()
            .map(|r| (r.size.w * r.size.h) as usize)
            .sum();
        assert!(covered < 300 * 300 / 2, "the hole was blurred: {covered}");
    }

    #[test]
    fn matches_point_set_difference() {
        let rects = [
            rect(0, 0, 40, 40),
            rect(30, 30, 40, 40),
            rect(5, 60, 10, 10),
            rect(50, 0, 20, 20),
        ];
        let others = [
            rect(10, 10, 20, 50),
            rect(35, 5, 30, 30),
            rect(0, 65, 80, 2),
        ];

        let expected: HashSet<_> = points(&rects)
            .difference(&points(&others))
            .copied()
            .collect();
        assert_eq!(points(&subtract(rects, others)), expected);
    }

    #[test]
    fn pieces_do_not_overlap() {
        let left = subtract(
            [rect(0, 0, 50, 50)],
            [rect(10, 10, 5, 5), rect(30, 0, 5, 50)],
        );
        let total: i32 = left.iter().map(|piece| piece.size.w * piece.size.h).sum();
        assert_eq!(total as usize, points(&left).len());
    }

    #[test]
    fn nothing_to_subtract_leaves_the_rects_alone() {
        let rects = [rect(0, 0, 10, 10), rect(20, 20, 5, 5)];
        assert_eq!(subtract(rects, []), rects.to_vec());
    }
}

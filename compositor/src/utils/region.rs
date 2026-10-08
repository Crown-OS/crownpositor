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

/// Disjoint rectangles covering the union of `rects` for drawing over: each
/// rectangle is folded into any piece their bounding box wastes little over,
/// overlaps are cut away so no pixel is covered twice, and a set that stays
/// fragmented falls back to one bounding box.
///
/// Folding as the rectangles arrive keeps this linear in the pieces kept. The
/// damage handed in can be hundreds of rectangles once glass in front has cut
/// it up, and searching every pair for the cheapest merge after every merge
/// cost more than the blur it was sizing.
pub fn coalesce(rects: impl IntoIterator<Item = Rect>) -> Vec<Rect> {
    let mut merged: Vec<Rect> = Vec::new();
    for rect in rects.into_iter().filter(|rect| !rect.is_empty()) {
        absorb(&mut merged, rect);
    }
    if merged.len() > MAX_PIECES {
        return bounding_box(merged);
    }

    let mut disjoint: Vec<Rect> = Vec::with_capacity(merged.len());
    for rect in merged {
        let pieces = subtract([rect], disjoint.iter().copied());
        disjoint.extend(pieces);
    }
    if disjoint.len() > MAX_PIECES {
        return bounding_box(disjoint);
    }
    disjoint
}

/// Adds `rect` to `pieces`, folded into every piece it merges with cheaply —
/// including those that only become cheap once it has grown.
fn absorb(pieces: &mut Vec<Rect>, mut rect: Rect) {
    while let Some(index) = pieces.iter().position(|piece| cheap_merge(*piece, rect)) {
        rect = rect.merge(pieces.swap_remove(index));
    }
    pieces.push(rect);
}

fn cheap_merge(a: Rect, b: Rect) -> bool {
    let area = |rect: Rect| f64::from(rect.size.w) * f64::from(rect.size.h);
    let overlap = a.intersection(b).map_or(0.0, area);
    let covered = area(a) + area(b) - overlap;
    area(a.merge(b)) <= MERGE_SLACK * covered.max(1.0)
}

fn bounding_box(rects: Vec<Rect>) -> Vec<Rect> {
    rects.into_iter().reduce(Rect::merge).into_iter().collect()
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

    /// What the overview hands a backdrop mid-drag: tile-shaped damage, cut
    /// up further by the glass in front.
    #[test]
    fn hundreds_of_overlapping_rectangles_collapse_into_a_few_pieces() {
        let rects: Vec<_> = (0..400)
            .map(|index| rect((index % 20) * 30, (index / 20) * 30, 140, 140))
            .collect();
        let coalesced = coalesce(rects.iter().copied());
        assert!(coalesced.len() <= MAX_PIECES, "{}", coalesced.len());

        let covered: usize = coalesced
            .iter()
            .map(|r| (r.size.w * r.size.h) as usize)
            .sum();
        assert_eq!(covered, points(&coalesced).len(), "pieces overlap");
        assert!(points(&rects).is_subset(&points(&coalesced)));
    }

    #[test]
    fn a_rectangle_bridging_two_pieces_folds_them_into_one() {
        let coalesced = coalesce([rect(0, 0, 10, 10), rect(20, 0, 10, 10), rect(0, 0, 30, 10)]);
        assert_eq!(coalesced, vec![rect(0, 0, 30, 10)]);
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

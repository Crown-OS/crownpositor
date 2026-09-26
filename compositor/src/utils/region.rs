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

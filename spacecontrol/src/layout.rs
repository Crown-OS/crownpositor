//! Arranging the overview's thumbnails.
//!
//! Windows are packed into rows, every row justified to the same width and
//! every window keeping its aspect ratio — the "justified gallery" shape. Rows
//! are contiguous in the order the caller passes windows in, so handing them
//! over in reading order keeps the overview roughly where the eye last left
//! them.
//!
//! The row count is not guessed. Every count from one row to one row per window
//! is laid out and the one leaving the largest thumbnails wins, which is what
//! makes a pair of windows sit side by side and a dozen fall into a grid
//! without either case being special.

use std::ops::Range;

use smithay::utils::{Logical, Rectangle, Size};

/// Width over height, guarding the degenerate sizes a client can commit before
/// it has been configured.
fn aspect(size: &Size<i32, Logical>) -> f64 {
    f64::from(size.w.max(1)) / f64::from(size.h.max(1))
}

/// Places every window inside `area`, leaving `gap` logical pixels between
/// neighbours and between rows. Results are index-aligned with `windows`.
///
/// `out` is a caller-owned buffer so a relayout reuses its allocation.
pub fn solve(
    area: Rectangle<i32, Logical>,
    windows: &[Size<i32, Logical>],
    gap: i32,
    out: &mut Vec<Rectangle<f64, Logical>>,
) {
    out.clear();
    if windows.is_empty() {
        return;
    }

    let gap = f64::from(gap);
    let mut best = (1, f64::NEG_INFINITY);
    for rows in 1..=windows.len() {
        let covered = place(area, windows, rows, gap, out);
        if covered > best.1 {
            best = (rows, covered);
        }
    }
    place(area, windows, best.0, gap, out);
}

/// Lays `windows` out in exactly `rows` rows and returns the area the
/// thumbnails cover, which is how [`solve`] compares one row count against
/// another.
fn place(
    area: Rectangle<i32, Logical>,
    windows: &[Size<i32, Logical>],
    rows: usize,
    gap: f64,
    out: &mut Vec<Rectangle<f64, Logical>>,
) -> f64 {
    let width = f64::from(area.size.w);
    let height = f64::from(area.size.h);

    // Each row's height is whatever makes that row exactly fill the width.
    let mut bounds: Vec<(Range<usize>, f64)> = Vec::with_capacity(rows);
    split(windows, rows, |range| {
        let span: f64 = windows[range.clone()].iter().map(aspect).sum();
        let usable = width - gap * (range.len() - 1) as f64;
        bounds.push((range, (usable / span).max(0.0)));
    });

    let gaps = gap * (bounds.len() - 1) as f64;
    let stacked: f64 = bounds.iter().map(|(_, height)| height).sum();
    // Three ceilings, and the lowest wins. The first fits the stack into the
    // height. The second is the width: a row height was chosen to fill the
    // width exactly, so anything above 1.0 pushes that row off both sides. The
    // third stops a lone window being blown up to fill the screen — measured
    // against the *largest* thumbnail, because measuring against the smallest
    // would let one tiny window drag every other thumbnail down with it.
    let natural = bounds
        .iter()
        .flat_map(|(range, row)| {
            windows[range.clone()]
                .iter()
                .map(move |size| f64::from(size.h) / row)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    let scale = ((height - gaps) / stacked).min(1.0).min(natural).max(0.0);

    out.clear();
    out.resize(windows.len(), Rectangle::default());

    let mut y = f64::from(area.loc.y) + (height - (stacked * scale + gaps)) / 2.0;
    let mut covered = 0.0;
    for (range, row) in bounds {
        let row = row * scale;
        let span: f64 = windows[range.clone()].iter().map(aspect).sum();
        let filled = span * row + gap * (range.len() - 1) as f64;

        let mut x = f64::from(area.loc.x) + (width - filled) / 2.0;
        for index in range {
            let width = aspect(&windows[index]) * row;
            out[index] = Rectangle::new((x, y).into(), (width, row).into());
            covered += width * row;
            x += width + gap;
        }
        y += row + gap;
    }
    covered
}

/// Cuts `windows` into `rows` contiguous, non-empty groups of roughly equal
/// total width, so no row ends up with a single window stretched across the
/// screen while the next is crowded.
fn split(windows: &[Size<i32, Logical>], rows: usize, mut row: impl FnMut(Range<usize>)) {
    let target: f64 = windows.iter().map(aspect).sum::<f64>() / rows as f64;

    let (mut start, mut span, mut emitted) = (0, 0.0, 0);
    for index in 0..windows.len() {
        span += aspect(&windows[index]);

        let left = windows.len() - index - 1;
        let rows_left = rows - emitted - 1;
        // Close on reaching this row's share, or as soon as every remaining
        // window is spoken for by a remaining row — whichever comes first keeps
        // the count exact and every row occupied.
        if (span >= target && rows_left > 0) || left == rows_left {
            row(start..index + 1);
            emitted += 1;
            start = index + 1;
            span = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), (1920, 1080).into())
    }

    fn sizes(of: &[(i32, i32)]) -> Vec<Size<i32, Logical>> {
        of.iter().map(|&(w, h)| Size::from((w, h))).collect()
    }

    fn solved(windows: &[(i32, i32)], gap: i32) -> Vec<Rectangle<f64, Logical>> {
        let mut out = Vec::new();
        solve(area(), &sizes(windows), gap, &mut out);
        out
    }

    fn overlaps(a: Rectangle<f64, Logical>, b: Rectangle<f64, Logical>) -> bool {
        // Touching edges are not an overlap, and the arithmetic lands a hair
        // either side of exact.
        let gap = 1e-6;
        a.loc.x + a.size.w - gap > b.loc.x
            && b.loc.x + b.size.w - gap > a.loc.x
            && a.loc.y + a.size.h - gap > b.loc.y
            && b.loc.y + b.size.h - gap > a.loc.y
    }

    fn assert_disjoint(rects: &[Rectangle<f64, Logical>]) {
        for (index, a) in rects.iter().enumerate() {
            for b in &rects[index + 1..] {
                assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
            }
        }
    }

    fn assert_inside(rects: &[Rectangle<f64, Logical>]) {
        for rect in rects {
            assert!(
                rect.loc.x >= -1e-6
                    && rect.loc.y >= -1e-6
                    && rect.loc.x + rect.size.w <= f64::from(area().size.w) + 1e-6
                    && rect.loc.y + rect.size.h <= f64::from(area().size.h) + 1e-6,
                "{rect:?} escapes the area"
            );
        }
    }

    #[test]
    fn no_windows_place_nothing() {
        assert!(solved(&[], 24).is_empty());
    }

    #[test]
    fn every_window_gets_a_rect() {
        assert_eq!(solved(&[(800, 600); 7], 24).len(), 7);
    }

    #[test]
    fn thumbnails_never_overlap() {
        // A deliberately awkward mix: a panorama, a portrait strip and a crowd
        // of ordinary windows.
        let windows = [
            (2560, 1080),
            (400, 1200),
            (1280, 720),
            (800, 600),
            (1920, 1080),
            (640, 480),
            (1000, 1000),
            (300, 900),
        ];
        for count in 1..=windows.len() {
            let rects = solved(&windows[..count], 24);
            assert_disjoint(&rects);
            assert_inside(&rects);
        }
    }

    #[test]
    fn aspect_ratios_survive() {
        let windows = [(2560, 1080), (400, 1200), (1280, 720), (640, 480)];
        for (rect, (w, h)) in solved(&windows, 24).iter().zip(windows) {
            let want = f64::from(w) / f64::from(h);
            let got = rect.size.w / rect.size.h;
            assert!((want - got).abs() < 1e-6, "{want} against {got}");
        }
    }

    #[test]
    fn a_lone_window_is_not_magnified() {
        let rect = solved(&[(800, 600)], 24)[0];
        assert!((rect.size.w - 800.0).abs() < 1e-6, "{rect:?}");
        assert!((rect.size.h - 600.0).abs() < 1e-6, "{rect:?}");
    }

    #[test]
    fn a_lone_oversized_window_is_shrunk_to_fit() {
        let rect = solved(&[(3840, 2160)], 24)[0];
        assert!(rect.size.w <= f64::from(area().size.w) + 1e-6, "{rect:?}");
        assert!(rect.size.h <= f64::from(area().size.h) + 1e-6, "{rect:?}");
        assert!((rect.size.w / rect.size.h - 16.0 / 9.0).abs() < 1e-6);
    }

    #[test]
    fn one_tiny_window_does_not_shrink_the_others() {
        // The no-magnify cap is measured against the largest thumbnail, so the
        // 200x150 window cannot drag the 1920x1080 one down with it.
        let rects = solved(&[(1920, 1080), (200, 150)], 24);
        assert!(rects[0].size.w > 800.0, "{:?}", rects[0]);
    }

    #[test]
    fn two_landscape_windows_sit_side_by_side() {
        let rects = solved(&[(1920, 1080), (1920, 1080)], 24);
        assert!((rects[0].loc.y - rects[1].loc.y).abs() < 1e-6, "{rects:?}");
        assert!(rects[0].loc.x < rects[1].loc.x);
    }

    #[test]
    fn many_windows_fall_into_more_than_one_row() {
        let rects = solved(&[(1280, 720); 9], 24);
        let rows: std::collections::BTreeSet<_> =
            rects.iter().map(|rect| rect.loc.y as i64).collect();
        assert!(rows.len() > 1, "nine windows should not be one row");
    }

    #[test]
    fn rows_keep_the_order_they_were_given() {
        let rects = solved(&[(1280, 720); 8], 24);
        for pair in rects.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(
                b.loc.y > a.loc.y - 1e-6 && (b.loc.y > a.loc.y + 1e-6 || b.loc.x > a.loc.x),
                "{a:?} then {b:?} is out of reading order"
            );
        }
    }

    #[test]
    fn the_block_is_centred_in_its_area() {
        let rects = solved(&[(1280, 720), (1280, 720)], 24);
        let left = rects.iter().map(|r| r.loc.x).fold(f64::MAX, f64::min);
        let right = rects
            .iter()
            .map(|r| r.loc.x + r.size.w)
            .fold(f64::MIN, f64::max);
        assert!(
            (left - (f64::from(area().size.w) - right)).abs() < 1e-6,
            "left {left}, right margin {}",
            f64::from(area().size.w) - right
        );
    }

    #[test]
    fn an_offset_area_moves_the_whole_block() {
        let windows = sizes(&[(1280, 720), (800, 600), (640, 480)]);
        let (mut origin, mut shifted) = (Vec::new(), Vec::new());
        solve(area(), &windows, 24, &mut origin);
        solve(
            Rectangle::new((300, 120).into(), area().size),
            &windows,
            24,
            &mut shifted,
        );

        for (a, b) in origin.iter().zip(&shifted) {
            assert!((b.loc.x - a.loc.x - 300.0).abs() < 1e-6, "{a:?} {b:?}");
            assert!((b.loc.y - a.loc.y - 120.0).abs() < 1e-6, "{a:?} {b:?}");
            assert!((a.size.w - b.size.w).abs() < 1e-6);
        }
    }

    #[test]
    fn the_gap_is_honoured_between_neighbours() {
        let rects = solved(&[(1280, 720), (1280, 720)], 40);
        assert!(
            (rects[1].loc.x - (rects[0].loc.x + rects[0].size.w) - 40.0).abs() < 1e-6,
            "{rects:?}"
        );
    }

    #[test]
    fn a_crowd_still_fits() {
        let rects = solved(&[(1920, 1080); 40], 16);
        assert_disjoint(&rects);
        assert_inside(&rects);
        assert!(rects.iter().all(|rect| rect.size.w > 1.0));
    }

    #[test]
    fn split_covers_every_window_exactly_once() {
        let windows = sizes(&[
            (1280, 720),
            (800, 600),
            (640, 480),
            (1920, 1080),
            (300, 900),
        ]);
        for rows in 1..=windows.len() {
            let mut ranges = Vec::new();
            split(&windows, rows, |range| ranges.push(range));

            assert_eq!(ranges.len(), rows, "{rows} rows requested");
            assert!(ranges.iter().all(|range| !range.is_empty()));
            assert_eq!(ranges[0].start, 0);
            assert_eq!(ranges[ranges.len() - 1].end, windows.len());
            for pair in ranges.windows(2) {
                assert_eq!(pair[0].end, pair[1].start);
            }
        }
    }
}

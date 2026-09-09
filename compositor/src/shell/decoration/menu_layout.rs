//! Where the menu row's labels and its open popup sit.
//!
//! Pure geometry, like the rest of this module, and shared by the renderer and
//! the hit test so the label you click is the one you see. Text widths are
//! *measured*, never assumed: the caller shapes the labels and hands their
//! widths in, because a menu laid out against guessed widths overlaps itself in
//! any font but the one it was tuned for.

use smithay::utils::{Logical, Point, Rectangle, Size};

/// Clear space either side of a menu label in the row.
const ROW_PADDING: i32 = 10;
/// A popup's own padding, top and bottom.
const POPUP_PADDING: i32 = 6;
/// Height of one popup row.
const ROW_HEIGHT: i32 = 28;
/// Height of a separator row, which is a rule with air around it.
const SEPARATOR_HEIGHT: i32 = 9;
/// Space from a popup's left edge to its labels.
const POPUP_INDENT: i32 = 14;
/// Space kept to the right of the longest label, for the submenu arrow.
const POPUP_GUTTER: i32 = 28;
/// Narrowest a popup may be, so a menu of one-letter items is still a target.
const MIN_POPUP_WIDTH: i32 = 160;

/// One entry in the menu row, and how wide its label came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowEntry {
    pub id: i32,
    /// The clickable rect, label padding included.
    pub rect: Rectangle<i32, Logical>,
    /// Where the label itself starts inside it.
    pub label: Point<i32, Logical>,
}

/// Lays out the top-level menu labels along the titlebar.
///
/// `labels` is `(id, measured width)` in the order they should appear. Entries
/// that would run past `available` are dropped rather than drawn over the
/// window controls.
pub fn row(
    available: Rectangle<i32, Logical>,
    labels: impl IntoIterator<Item = (i32, i32)>,
) -> Vec<RowEntry> {
    let mut entries = Vec::new();
    let mut x = available.loc.x;

    for (id, width) in labels {
        let full = width + ROW_PADDING * 2;
        if x + full > available.loc.x + available.size.w {
            break;
        }
        entries.push(RowEntry {
            id,
            rect: Rectangle::new(
                Point::from((x, available.loc.y)),
                Size::from((full, available.size.h)),
            ),
            label: Point::from((x + ROW_PADDING, available.loc.y)),
        });
        x += full;
    }

    entries
}

/// Which row entry a point is in.
pub fn entry_at(entries: &[RowEntry], point: Point<f64, Logical>) -> Option<i32> {
    entries
        .iter()
        .find(|entry| entry.rect.to_f64().contains(point))
        .map(|entry| entry.id)
}

/// One row of an open popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupRow {
    pub id: i32,
    pub rect: Rectangle<i32, Logical>,
    /// `false` for a separator, which is drawn but never hit.
    pub actionable: bool,
}

/// An open popup: its frame and the rows inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    pub frame: Rectangle<i32, Logical>,
    pub rows: Vec<PopupRow>,
}

impl Popup {
    pub fn row_at(&self, point: Point<f64, Logical>) -> Option<i32> {
        self.rows
            .iter()
            .find(|row| row.actionable && row.rect.to_f64().contains(point))
            .map(|row| row.id)
    }

    /// Where the label of a row starts.
    pub fn label_origin(&self, row: &PopupRow) -> Point<i32, Logical> {
        Point::from((row.rect.loc.x + POPUP_INDENT, row.rect.loc.y))
    }
}

/// Builds a popup hanging below `anchor`, kept inside `bounds`.
///
/// `items` is `(id, measured label width, is separator)` in order.
pub fn popup(
    anchor: Point<i32, Logical>,
    bounds: Rectangle<i32, Logical>,
    items: &[(i32, i32, bool)],
) -> Popup {
    let width = items
        .iter()
        .map(|(_, width, _)| *width + POPUP_INDENT + POPUP_GUTTER)
        .max()
        .unwrap_or(MIN_POPUP_WIDTH)
        .max(MIN_POPUP_WIDTH)
        .min(bounds.size.w.max(MIN_POPUP_WIDTH));

    let height = POPUP_PADDING * 2
        + items
            .iter()
            .map(|(_, _, separator)| row_height(*separator))
            .sum::<i32>();

    // Flipped up when there is no room below, and pulled back inside when it
    // would run off the right — the two things every menu on every desktop
    // does at the edge of a screen.
    let x = anchor
        .x
        .min(bounds.loc.x + bounds.size.w - width)
        .max(bounds.loc.x);
    let below = anchor.y;
    let y = if below + height <= bounds.loc.y + bounds.size.h {
        below
    } else {
        (anchor.y - height).max(bounds.loc.y)
    };

    let frame = Rectangle::new(Point::from((x, y)), Size::from((width, height)));

    let mut rows = Vec::with_capacity(items.len());
    let mut offset = y + POPUP_PADDING;
    for (id, _, separator) in items {
        let extent = row_height(*separator);
        rows.push(PopupRow {
            id: *id,
            rect: Rectangle::new(Point::from((x, offset)), Size::from((width, extent))),
            actionable: !*separator,
        });
        offset += extent;
    }

    Popup { frame, rows }
}

fn row_height(separator: bool) -> i32 {
    if separator {
        SEPARATOR_HEIGHT
    } else {
        ROW_HEIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn middle(rect: Rectangle<i32, Logical>) -> Point<f64, Logical> {
        Point::from((
            rect.loc.x as f64 + rect.size.w as f64 / 2.0,
            rect.loc.y as f64 + rect.size.h as f64 / 2.0,
        ))
    }

    #[test]
    fn labels_run_left_to_right_without_overlapping() {
        let entries = row(area(100, 0, 400, 36), [(1, 30), (2, 30), (3, 60)]);

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].rect.loc.x, 100);
        for pair in entries.windows(2) {
            assert_eq!(pair[1].rect.loc.x, pair[0].rect.loc.x + pair[0].rect.size.w);
        }
    }

    #[test]
    fn a_label_is_padded_on_both_sides() {
        let entries = row(area(0, 0, 400, 36), [(1, 40)]);
        assert_eq!(entries[0].rect.size.w, 40 + ROW_PADDING * 2);
        assert_eq!(entries[0].label.x, ROW_PADDING);
    }

    /// Better to show three menus than to draw the fourth over the close
    /// button.
    #[test]
    fn entries_that_do_not_fit_are_dropped() {
        let entries = row(area(0, 0, 100, 36), [(1, 30), (2, 30), (3, 30)]);
        assert_eq!(entries.len(), 2);
        let last = entries.last().unwrap();
        assert!(last.rect.loc.x + last.rect.size.w <= 100);
    }

    #[test]
    fn no_labels_means_no_row() {
        assert!(row(area(0, 0, 400, 36), []).is_empty());
    }

    #[test]
    fn clicking_a_label_finds_it() {
        let entries = row(area(0, 0, 400, 36), [(1, 30), (2, 30)]);
        assert_eq!(entry_at(&entries, (5.0, 10.0).into()), Some(1));
        assert_eq!(entry_at(&entries, (60.0, 10.0).into()), Some(2));
        assert_eq!(entry_at(&entries, (300.0, 10.0).into()), None);
    }

    #[test]
    fn a_popup_hangs_below_its_anchor() {
        let popup = popup((100, 36).into(), area(0, 0, 1000, 800), &[(1, 60, false)]);
        assert_eq!(popup.frame.loc, (100, 36).into());
        assert_eq!(popup.rows[0].rect.loc.y, 36 + POPUP_PADDING);
    }

    #[test]
    fn rows_stack_and_stay_inside_the_frame() {
        let popup = popup(
            (0, 0).into(),
            area(0, 0, 1000, 800),
            &[(1, 60, false), (2, 60, false), (3, 60, false)],
        );

        for pair in popup.rows.windows(2) {
            assert_eq!(pair[1].rect.loc.y, pair[0].rect.loc.y + pair[0].rect.size.h);
        }
        let last = popup.rows.last().unwrap();
        assert!(last.rect.loc.y + last.rect.size.h <= popup.frame.loc.y + popup.frame.size.h);
    }

    #[test]
    fn a_separator_is_shorter_and_not_clickable() {
        let popup = popup(
            (0, 0).into(),
            area(0, 0, 1000, 800),
            &[(1, 60, false), (2, 0, true)],
        );

        assert!(popup.rows[1].rect.size.h < popup.rows[0].rect.size.h);
        assert!(!popup.rows[1].actionable);
        assert_eq!(
            popup.row_at(middle(popup.rows[1].rect)),
            None,
            "a rule is not a target"
        );
    }

    #[test]
    fn the_widest_label_sets_the_width() {
        let narrow = popup((0, 0).into(), area(0, 0, 1000, 800), &[(1, 10, false)]);
        let wide = popup((0, 0).into(), area(0, 0, 1000, 800), &[(1, 400, false)]);
        assert!(wide.frame.size.w > narrow.frame.size.w);
        assert!(narrow.frame.size.w >= MIN_POPUP_WIDTH);
    }

    #[test]
    fn a_popup_at_the_right_edge_is_pulled_back_inside() {
        let popup = popup((980, 36).into(), area(0, 0, 1000, 800), &[(1, 200, false)]);
        assert!(popup.frame.loc.x + popup.frame.size.w <= 1000);
        assert!(popup.frame.loc.x >= 0);
    }

    /// A window near the bottom of the screen opens its menu upward.
    #[test]
    fn a_popup_with_no_room_below_flips_above_its_anchor() {
        let rows: Vec<_> = (1..=10).map(|id| (id, 60, false)).collect();
        let popup = popup((0, 780).into(), area(0, 0, 1000, 800), &rows);

        assert!(popup.frame.loc.y < 780, "it opened upward");
        assert!(popup.frame.loc.y >= 0);
    }

    #[test]
    fn clicking_a_row_finds_it() {
        let popup = popup(
            (0, 0).into(),
            area(0, 0, 1000, 800),
            &[(1, 60, false), (2, 60, false)],
        );

        assert_eq!(popup.row_at(middle(popup.rows[0].rect)), Some(1));
        assert_eq!(popup.row_at(middle(popup.rows[1].rect)), Some(2));
        assert_eq!(popup.row_at((500.0, 500.0).into()), None);
    }
}

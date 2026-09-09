//! Where a floating window goes, and how it is kept on screen.
//!
//! The size a window opens at is the client's own: a floating workspace is not
//! a layout, and a compositor that picks the size defeats the point. Only the
//! *position* is decided here, and only the position is corrected afterwards.

use smithay::utils::{Logical, Point, Rectangle, Size};

/// Offset between successive cascaded windows.
const CASCADE_STEP: i32 = 32;
/// How far a cascade walks before returning to the top left.
const CASCADE_LIMIT: i32 = 8;
/// What a window opens at when the client committed no geometry at all.
const FALLBACK_RATIO: f64 = 0.6;
/// How much of a window has to stay inside the usable area to be caught again
/// — roughly a thumb's width of titlebar, enough to aim at without hunting.
const REACHABLE: i32 = 64;

/// The rect a newly floated window opens at, decoration included.
///
/// `content` is the size the client committed, and a zero component means it
/// has not said — the only case where the compositor picks one. `decoration` is
/// the height of the band drawn above it, added *after* that decision: a client
/// that has committed nothing yet reports zero, and a zero plus a titlebar
/// looks exactly like a window that asked to be one titlebar tall.
pub fn initial_rect(
    content: Size<i32, Logical>,
    decoration: i32,
    area: Rectangle<i32, Logical>,
    parent: Option<Rectangle<i32, Logical>>,
    cascade: usize,
) -> Rectangle<i32, Logical> {
    let available = Size::from((area.size.w, (area.size.h - decoration).max(1)));
    let content = honour(content, available);
    place(
        Size::from((content.w, content.h + decoration)),
        area,
        parent,
        cascade,
    )
}

/// Where a floating window of `size` goes.
///
/// Centred on its parent when it has one — a dialog belongs over the window that
/// opened it — otherwise cascaded from the top left so a burst of windows does
/// not land in one stack.
pub fn place(
    size: Size<i32, Logical>,
    area: Rectangle<i32, Logical>,
    parent: Option<Rectangle<i32, Logical>>,
    cascade: usize,
) -> Rectangle<i32, Logical> {
    // A window larger than the screen is capped: there is nowhere to put the
    // overhang, and the client will be told the size it actually got.
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

/// Holds back the strip of a window the user catches it by.
///
/// Not [`clamp_into`]: a dragged window may hang as far off an edge as the
/// pointer takes it, and only enough to grab is kept inside. Without a floor a
/// window dragged past the bottom is gone for good — there is nothing left on
/// screen to click, and no way to bring it back.
///
/// The top is the exception and is held completely: a titlebar above the usable
/// area is behind a panel or off the screen, and either way cannot be clicked.
pub fn keep_reachable(
    rect: Rectangle<i32, Logical>,
    area: Rectangle<i32, Logical>,
    decoration: i32,
) -> Rectangle<i32, Logical> {
    // What the window is caught by: its titlebar, or a strip of the window
    // itself when it has none. Never taller than the window.
    let band = if decoration > 0 {
        decoration
    } else {
        REACHABLE
    };
    let grip = band.min(rect.size.h.max(1));
    // A window narrower than the margin has none of itself to spare, so it
    // stays inside altogether.
    let across = REACHABLE.min(rect.size.w);

    let min_x = area.loc.x + across - rect.size.w;
    let max_x = (area.loc.x + area.size.w - across).max(min_x);
    let min_y = area.loc.y;
    let max_y = (area.loc.y + area.size.h - grip).max(min_y);

    Rectangle::new(
        Point::from((
            rect.loc.x.clamp(min_x, max_x),
            rect.loc.y.clamp(min_y, max_y),
        )),
        rect.size,
    )
}

/// Where a window pulled off a snap goes, so the cursor keeps its grip.
///
/// A snapped window restores to a different size, and dropping it back at the
/// rect it last floated at leaves the pointer somewhere else entirely — usually
/// off the window altogether. The grab point keeps its position *along* the
/// titlebar instead: grabbed a third of the way across, it stays a third of the
/// way across whatever width the window returns to.
///
/// The top edge does not move, because the titlebar is the same height either
/// way and the cursor is already somewhere inside it.
pub fn keep_grip(
    restored: Size<i32, Logical>,
    from: Rectangle<i32, Logical>,
    pointer: Point<f64, Logical>,
) -> Rectangle<i32, Logical> {
    // A window with no width to speak of has no meaningful grab fraction, so
    // the pointer lands in the middle of the restored one.
    let across = if from.size.w > 0 {
        ((pointer.x - from.loc.x as f64) / from.size.w as f64).clamp(0.0, 1.0)
    } else {
        0.5
    };

    Rectangle::new(
        Point::from((
            (pointer.x - across * restored.w as f64).round() as i32,
            from.loc.y,
        )),
        restored,
    )
}

/// Stops a resize dragging a window's top edge out of reach, keeping the
/// opposite edge where the user left it.
///
/// [`keep_reachable`] cannot do this job: it moves a window without resizing
/// it, and moving one mid-resize would drag the anchored edge along too.
pub fn keep_top_reachable(
    rect: Rectangle<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let overshoot = (area.loc.y - rect.loc.y).max(0);
    Rectangle::new(
        Point::from((rect.loc.x, rect.loc.y + overshoot)),
        Size::from((rect.size.w, (rect.size.h - overshoot).max(1))),
    )
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

/// The client's size where it gave one, a fraction of the area where it did not.
fn honour(requested: Size<i32, Logical>, area: Size<i32, Logical>) -> Size<i32, Logical> {
    let fallback = |extent: i32| ((extent as f64 * FALLBACK_RATIO).round() as i32).max(1);
    Size::from((
        if requested.w > 0 {
            requested.w
        } else {
            fallback(area.w)
        },
        if requested.h > 0 {
            requested.h
        } else {
            fallback(area.h)
        },
    ))
}

fn center_in(within: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
    Point::from((
        within.loc.x + (within.size.w - size.w) / 2,
        within.loc.y + (within.size.h - size.h) / 2,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::testing::area;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_window_opens_at_the_size_it_asked_for() {
        let rect = initial_rect((640, 480).into(), 0, area(1920, 1080), None, 0);
        assert_eq!(rect.size, (640, 480).into());
    }

    #[test]
    fn a_client_that_names_no_size_gets_a_fraction_of_the_area() {
        let rect = initial_rect((0, 0).into(), 0, area(1000, 1000), None, 0);
        assert_eq!(rect.size, (600, 600).into());
    }

    #[test]
    fn one_unset_axis_does_not_discard_the_other() {
        let rect = initial_rect((800, 0).into(), 0, area(1000, 1000), None, 0);
        assert_eq!(rect.size, (800, 600).into());
    }

    #[test]
    fn a_decorated_window_grows_by_its_titlebar() {
        let rect = initial_rect((640, 480).into(), 36, area(1920, 1080), None, 0);
        assert_eq!(rect.size, (640, 516).into());
    }

    /// The case that mattered: a client with no committed geometry reports
    /// zero, and adding the titlebar before resolving that would open the
    /// window at exactly one titlebar tall.
    #[test]
    fn an_unset_size_is_resolved_before_the_titlebar_is_added() {
        let rect = initial_rect((0, 0).into(), 36, area(1000, 1000), None, 0);
        assert!(
            rect.size.h > 36 * 2,
            "opened {}px tall, which is a titlebar and nothing else",
            rect.size.h
        );
        assert_eq!(rect.size, (600, 614).into());
    }

    #[test]
    fn a_dialog_centers_on_its_parent() {
        let parent = Rectangle::new((100, 100).into(), (600, 400).into());
        let rect = place((200, 100).into(), area(1920, 1080), Some(parent), 0);
        assert_eq!(rect.loc, (300, 250).into());
    }

    #[test]
    fn parentless_windows_cascade() {
        let first = place((400, 300).into(), area(1920, 1080), None, 0);
        let second = place((400, 300).into(), area(1920, 1080), None, 1);
        assert_ne!(first.loc, second.loc);
        assert_eq!(second.loc.y - first.loc.y, CASCADE_STEP);
    }

    #[test]
    fn the_cascade_wraps_instead_of_walking_off_screen() {
        let first = place((400, 300).into(), area(1920, 1080), None, 0);
        let wrapped = place((400, 300).into(), area(1920, 1080), None, 8);
        assert_eq!(first.loc, wrapped.loc);
    }

    #[test]
    fn placement_stays_inside_the_area() {
        let parent = Rectangle::new((1800, 1000).into(), (100, 60).into());
        let rect = place((600, 400).into(), area(1920, 1080), Some(parent), 0);
        assert!(rect.loc.x >= 0 && rect.loc.y >= 0);
        assert!(rect.loc.x + rect.size.w <= 1920);
        assert!(rect.loc.y + rect.size.h <= 1080);
    }

    #[test]
    fn oversized_windows_are_capped_to_the_area() {
        let rect = place((2000, 2000).into(), area(800, 600), None, 0);
        assert_eq!(rect.size, (800, 600).into());
    }

    #[test]
    fn a_window_dragged_off_a_side_keeps_a_strip_on_screen() {
        let far_left = keep_reachable(rect(-5000, 100, 400, 300), area(1000, 800), 36);
        assert_eq!(far_left.loc.x + far_left.size.w, REACHABLE);

        let far_right = keep_reachable(rect(5000, 100, 400, 300), area(1000, 800), 36);
        assert_eq!(far_right.loc.x, 1000 - REACHABLE);
    }

    #[test]
    fn a_window_dragged_off_the_bottom_keeps_its_titlebar() {
        let sunk = keep_reachable(rect(100, 5000, 400, 300), area(1000, 800), 36);
        assert_eq!(sunk.loc.y, 800 - 36, "the whole titlebar stays visible");
    }

    /// A titlebar above the usable area is behind a panel or off the screen,
    /// and either way there is nothing left to click.
    #[test]
    fn a_window_never_rises_above_the_usable_area() {
        let area = Rectangle::new((8, 48).into(), (984, 744).into());
        assert_eq!(
            keep_reachable(rect(100, -300, 400, 300), area, 36).loc.y,
            48
        );
    }

    #[test]
    fn a_window_already_on_screen_is_left_alone() {
        let inside = rect(100, 100, 400, 300);
        assert_eq!(keep_reachable(inside, area(1000, 800), 36), inside);
    }

    #[test]
    fn keeping_a_window_reachable_never_resizes_it() {
        let dragged = rect(-5000, 5000, 400, 300);
        assert_eq!(
            keep_reachable(dragged, area(1000, 800), 36).size,
            dragged.size
        );
    }

    /// Nothing to spare, so it stays inside altogether rather than being
    /// allowed to hang off by more than its own width.
    #[test]
    fn a_window_narrower_than_the_margin_stays_wholly_inside() {
        let tiny = keep_reachable(rect(-500, 100, 40, 300), area(1000, 800), 36);
        assert_eq!(tiny.loc.x, 0);
    }

    /// An area smaller than the margin inverts the bounds, and `clamp` panics
    /// on an inverted range.
    #[test]
    fn an_area_smaller_than_the_margin_does_not_panic() {
        let squeezed = keep_reachable(rect(5000, 5000, 400, 300), area(20, 20), 36);
        assert_eq!(squeezed.loc, (20 - REACHABLE, 0).into());
    }

    /// The whole point: the pointer is over the same part of the titlebar
    /// after the restore as it was before, so the window does not leap out from
    /// under the cursor.
    #[test]
    fn a_window_pulled_off_a_snap_stays_under_the_cursor() {
        // Snapped to the left half, grabbed halfway across it.
        let snapped = rect(0, 48, 500, 700);
        let pointer = (250.0, 60.0).into();

        let restored = keep_grip((300, 200).into(), snapped, pointer);

        assert_eq!(restored.size, (300, 200).into());
        assert_eq!(restored.loc.x, 250 - 150, "still halfway across");
        assert!(
            (restored.loc.x..restored.loc.x + restored.size.w).contains(&250),
            "the cursor has to still be on the window"
        );
    }

    #[test]
    fn the_grab_keeps_its_fraction_rather_than_its_offset() {
        let snapped = rect(0, 0, 1000, 800);
        // A tenth of the way across a wide window.
        let restored = keep_grip((200, 200).into(), snapped, (100.0, 10.0).into());
        assert_eq!(restored.loc.x, 100 - 20);
    }

    #[test]
    fn the_top_edge_does_not_move_when_a_snap_is_released() {
        let snapped = rect(0, 400, 500, 400);
        assert_eq!(
            keep_grip((300, 200).into(), snapped, (100.0, 410.0).into())
                .loc
                .y,
            400
        );
    }

    #[test]
    fn a_grip_on_a_window_with_no_width_lands_in_the_middle() {
        let restored = keep_grip((300, 200).into(), rect(0, 0, 0, 0), (500.0, 10.0).into());
        assert_eq!(restored.loc.x, 500 - 150);
    }

    /// A window that never floated restores to the size it already has, so the
    /// rect must come back unchanged — no jump at all.
    #[test]
    fn regripping_at_the_same_size_changes_nothing() {
        let current = rect(120, 240, 500, 400);
        let pointer = (300.0, 250.0).into();
        assert_eq!(keep_grip(current.size, current, pointer), current);
    }

    #[test]
    fn a_resize_cannot_pull_the_top_edge_out_of_reach() {
        let area = Rectangle::new((8, 48).into(), (984, 744).into());
        let held = keep_top_reachable(rect(100, -52, 400, 500), area);

        assert_eq!(held.loc.y, 48, "the top stops at the usable area");
        assert_eq!(
            held.loc.y + held.size.h,
            -52 + 500,
            "and the edge being dragged against stays put"
        );
    }

    #[test]
    fn a_resize_inside_the_area_is_left_alone() {
        let area = Rectangle::new((8, 48).into(), (984, 744).into());
        let inside = rect(100, 100, 400, 300);
        assert_eq!(keep_top_reachable(inside, area), inside);
    }

    #[test]
    fn a_resize_held_at_the_top_never_collapses_to_nothing() {
        let area = Rectangle::new((0, 500).into(), (1000, 300).into());
        assert!(keep_top_reachable(rect(0, 0, 400, 10), area).size.h >= 1);
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

//! Where the panel and every card in it sit, for one output at one moment.
//!
//! The row is a stack laid on its side: the selected window is the front card
//! at the row's leading edge, the ones before it are tucked behind it — each a
//! little smaller and peeking out a little further left — and the ones after it
//! follow in a line. Advancing slides the next card forward over the stack.
//! Every card's corners are concentric with the panel's.

use smithay::utils::{Logical, Point, Rectangle, Size};
use spacecontrol::scene::Canvas;

use crate::strip::Strip;

/// Width over height a card may take, whatever shape its window is.
const ASPECTS: (f64, f64) = (0.6, 2.2);

/// Proportions of the output height, so the panel looks the same on every
/// output.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// The front card's height.
    pub card: f64,
    pub gap: f64,
    /// Between the panel's edge and the cards, all the way round.
    pub padding: f64,
    /// Between the panel and the bottom of the usable area.
    pub margin: f64,
    /// The panel's corner radius over its height.
    pub roundness: f64,
    /// How much smaller a card is while another is selected, as a fraction.
    pub recede: f64,
    /// How much a card fades while another is selected, as a fraction.
    pub fade: f64,
    /// How far each card stacked behind the front one peeks out.
    pub peek: f64,
    /// How much smaller each card further back in the stack is, as a fraction.
    pub stack_shrink: f64,
    /// How many cards the stack shows; one deeper sinks out of sight.
    pub stack_depth: f64,
    /// How much a hovered card grows, as a fraction.
    pub hover: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            card: 0.16,
            gap: 0.014,
            padding: 0.014,
            margin: 0.03,
            roundness: 0.24,
            recede: 0.12,
            fade: 0.3,
            peek: 0.012,
            stack_shrink: 0.06,
            stack_depth: 3.0,
            hover: 0.03,
        }
    }
}

/// The glass the cards sit on.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Panel {
    pub rect: Rectangle<f64, Logical>,
    pub radius: f64,
    /// What the cards are cut to: the panel less its padding either side, so
    /// a card running off the end never crosses the panel's rounded corner.
    pub row: Rectangle<f64, Logical>,
}

/// A card as drawn: where, how opaque and how round.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub rect: Rectangle<f64, Logical>,
    pub alpha: f64,
    pub radius: f64,
}

/// Lays out the panel and one slot per entry, and returns the panel. `reveal`
/// is the raw rise, so an overshoot lifts the panel past its resting place; at
/// 0 it sits just below the usable area. Slots come in entry order, which back
/// to front is also drawing order: the stack deepens towards the first entry.
pub fn arrange<K: Copy + Eq>(
    canvas: Canvas,
    metrics: &Metrics,
    reveal: f64,
    strip: &Strip<K>,
    aspect: impl Fn(K) -> f64,
    out: &mut Vec<Slot>,
) -> Panel {
    let unit = f64::from(canvas.output.size.h);
    let usable = canvas.usable.to_f64();
    let padding = unit * metrics.padding;
    let height = unit * metrics.card + 2.0 * padding;
    let radius = height * metrics.roundness;

    row(unit, metrics, radius - padding, strip, aspect, out);
    let (left, right) = extent(out);
    let widest = (usable.size.w - 2.0 * unit * metrics.margin).max(height);
    let width = (right - left + 2.0 * padding).clamp(height, widest);

    let bottom = usable.loc.y + usable.size.h;
    let resting = bottom - unit * metrics.margin - height;
    let rect = Rectangle::new(
        (
            usable.loc.x + (usable.size.w - width) / 2.0,
            bottom + (resting - bottom) * reveal,
        )
            .into(),
        (width, height).into(),
    );
    let row = Rectangle::new(
        (rect.loc.x + padding, rect.loc.y).into(),
        (width - 2.0 * padding, height).into(),
    );

    let origin = Point::from((row.loc.x - left, rect.loc.y + height / 2.0));
    let end = row.loc.x + row.size.w;
    for slot in out.iter_mut() {
        slot.rect.loc += origin;
        slot.alpha *= ((end - slot.rect.loc.x) / slot.rect.size.w).clamp(0.0, 1.0);
    }
    Panel { rect, radius, row }
}

/// The cards around a front card whose leading edge is at 0 and whose middle
/// is on the x axis.
fn row<K: Copy + Eq>(
    unit: f64,
    metrics: &Metrics,
    radius: f64,
    strip: &Strip<K>,
    aspect: impl Fn(K) -> f64,
    out: &mut Vec<Slot>,
) {
    out.clear();
    let card = unit * metrics.card;
    let gap = unit * metrics.gap;
    let position = strip.position();

    let mut x = 0.0;
    let mut anchors = (0.0, 0.0);
    for (index, entry) in strip.entries().iter().enumerate() {
        let presence = entry.presence();
        let background = 1.0 - entry.focus();
        let scale =
            presence * (1.0 - metrics.recede * background) * (1.0 + metrics.hover * entry.lift());
        let size = Size::from((
            card * aspect(entry.key).clamp(ASPECTS.0, ASPECTS.1) * scale,
            card * scale,
        ));
        if index == position.floor() as usize {
            anchors.0 = x;
        }
        if index == position.ceil() as usize {
            anchors.1 = x;
        }
        out.push(Slot {
            rect: Rectangle::new((x, 0.0).into(), size),
            alpha: presence * (1.0 - metrics.fade * background),
            radius: radius * scale,
        });
        x += size.w + gap * presence;
    }

    let anchor = anchors.0 + (anchors.1 - anchors.0) * position.fract();
    for (index, slot) in out.iter_mut().enumerate() {
        let depth = position - index as f64;
        if depth > 0.0 {
            tuck(slot, unit, metrics, depth);
        } else {
            slot.rect.loc.x -= anchor;
        }
        slot.rect.loc.y = -slot.rect.size.h / 2.0;
    }
}

/// Moves a card `depth` places back into the stack. The deepest place shown
/// is as far left as the stack reaches: a card going deeper still shrinks and
/// fades where it is rather than pushing the panel wider.
fn tuck(slot: &mut Slot, unit: f64, metrics: &Metrics, depth: f64) {
    let shrink = (1.0 - metrics.stack_shrink * depth).max(0.0);
    slot.rect.size = slot.rect.size.upscale(shrink);
    slot.rect.loc.x = -unit * metrics.peek * depth.min(metrics.stack_depth - 1.0);
    slot.radius *= shrink;
    slot.alpha *= (metrics.stack_depth - depth).clamp(0.0, 1.0);
}

/// The leftmost and rightmost edges of anything still showing.
fn extent(slots: &[Slot]) -> (f64, f64) {
    slots
        .iter()
        .filter(|slot| slot.alpha > 0.0)
        .fold((0.0, 0.0), |(left, right), slot| {
            (
                f64::min(left, slot.rect.loc.x),
                f64::max(right, slot.rect.loc.x + slot.rect.size.w),
            )
        })
}

/// The frontmost card under `at`, if it is inside the panel's row.
pub fn slot_at(panel: &Panel, slots: &[Slot], at: Point<f64, Logical>) -> Option<usize> {
    if !panel.row.contains(at) {
        return None;
    }
    slots
        .iter()
        .rposition(|slot| slot.alpha > 0.5 && slot.rect.contains(at))
}

/// `rect` stepped back towards `point` by `factor`, as the windows behind the
/// panel are.
pub fn recede(
    rect: Rectangle<f64, Logical>,
    point: Point<f64, Logical>,
    factor: f64,
) -> Rectangle<f64, Logical> {
    Rectangle::new(
        (
            point.x + (rect.loc.x - point.x) * factor,
            point.y + (rect.loc.y - point.y) * factor,
        )
            .into(),
        (rect.size.w * factor, rect.size.h * factor).into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strip::Direction;

    fn canvas() -> Canvas {
        Canvas::whole(Rectangle::new((0, 0).into(), (1920, 1080).into()))
    }

    fn laid_out(keys: &[u32], direction: Direction, reveal: f64) -> (Panel, Vec<Slot>) {
        let mut strip = Strip::default();
        strip.reset(keys.iter().copied(), direction);
        let mut out = Vec::new();
        let panel = arrange(
            canvas(),
            &Metrics::default(),
            reveal,
            &strip,
            |_| 16.0 / 10.0,
            &mut out,
        );
        (panel, out)
    }

    #[test]
    fn the_hidden_panel_sits_below_the_usable_area() {
        let (panel, _) = laid_out(&[1, 2], Direction::Forward, 0.0);
        assert_eq!(panel.rect.loc.y, 1080.0);
    }

    #[test]
    fn the_risen_panel_rests_on_the_bottom_of_the_screen() {
        let (panel, _) = laid_out(&[1, 2], Direction::Forward, 1.0);
        assert!(panel.rect.loc.y + panel.rect.size.h < 1080.0);
        assert!(panel.rect.loc.y > 1080.0 * 0.7);
    }

    #[test]
    fn the_selected_card_is_in_front_and_largest() {
        let (_, slots) = laid_out(&[1, 2, 3], Direction::Forward, 1.0);
        let front = slots[1].rect;
        assert!(slots.iter().all(|slot| slot.rect.size.h <= front.size.h));
        assert!(slots[1].alpha > slots[2].alpha);
    }

    #[test]
    fn earlier_cards_are_tucked_behind_and_peek_out_left() {
        let (_, slots) = laid_out(&[1, 2, 3], Direction::Backward, 1.0);
        let (deepest, behind, front) = (slots[0].rect, slots[1].rect, slots[2].rect);
        assert!(deepest.loc.x < behind.loc.x && behind.loc.x < front.loc.x);
        assert!(deepest.size.h < behind.size.h && behind.size.h < front.size.h);
        assert!(behind.overlaps(front));
    }

    #[test]
    fn later_cards_follow_in_a_line() {
        let (_, slots) = laid_out(&[1, 2, 3], Direction::Forward, 1.0);
        assert!(slots[1].rect.loc.x + slots[1].rect.size.w < slots[2].rect.loc.x);
    }

    #[test]
    fn the_front_card_is_concentric_with_the_panel() {
        let (panel, slots) = laid_out(&[1, 2], Direction::Forward, 1.0);
        let front = slots[1];
        let inset = front.rect.loc.y - panel.rect.loc.y;
        assert!((panel.radius - inset - front.radius).abs() < 1e-6);
        assert!(
            (panel.rect.loc.y + panel.rect.size.h - front.rect.loc.y - front.rect.size.h - inset)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn the_panel_hugs_a_short_row() {
        let (panel, slots) = laid_out(&[1, 2], Direction::Backward, 1.0);
        assert!(panel.rect.size.w < 1000.0, "{}", panel.rect.size.w);
        for slot in slots {
            assert!(panel.rect.contains_rect(slot.rect));
        }
    }

    #[test]
    fn a_long_row_fades_out_at_the_usable_width() {
        let keys: Vec<_> = (0..40).collect();
        let (panel, slots) = laid_out(&keys, Direction::Forward, 1.0);
        assert!(panel.rect.size.w < 1920.0);
        assert_eq!(slots.last().map(|slot| slot.alpha), Some(0.0));
    }

    #[test]
    fn a_click_finds_the_front_card_over_the_stack() {
        let (panel, slots) = laid_out(&[1, 2, 3], Direction::Backward, 1.0);
        let front = spacecontrol::scene::centre(slots[2].rect);
        assert_eq!(slot_at(&panel, &slots, front), Some(2));
        let peek = Point::from((slots[0].rect.loc.x + 1.0, front.y));
        assert_eq!(slot_at(&panel, &slots, peek), Some(0));
        assert_eq!(slot_at(&panel, &slots, (0.0, 0.0).into()), None);
    }

    #[test]
    fn receding_keeps_the_vanishing_point_still() {
        let rect = Rectangle::new((100.0, 100.0).into(), (800.0, 600.0).into());
        let point = Point::from((500.0, 400.0));
        let receded = recede(rect, point, 0.5);
        assert_eq!(receded.loc, Point::from((300.0, 250.0)));
        assert_eq!(receded.size, (400.0, 300.0).into());
    }
}

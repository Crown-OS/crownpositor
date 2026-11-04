//! Where every card sits, for one output at one moment.
//!
//! The cards are a stack seen from the front. The selected window is the top
//! card, in the middle of the screen; the windows before it are stacked behind
//! it to the left and the ones after it to the right, each a little smaller and
//! peeking out a little further than the card in front of it. Advancing slides
//! the next card out of the stack and over the rest.

use smithay::utils::{Logical, Point, Rectangle, Size};
use spacecontrol::scene::Canvas;

use crate::strip::Strip;

/// Width over height a card may take, whatever shape its window is.
const ASPECTS: (f64, f64) = (0.5, 2.4);

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// The largest the top card may be, as fractions of the usable width and
    /// height.
    pub card: (f64, f64),
    /// How far below the middle of the usable area the cards sit, over the
    /// output height: room for the titles above them.
    pub drop: f64,
    /// A card's corner radius over the output height.
    pub roundness: f64,
    /// How far each card peeks out past the one in front of it, over the
    /// output height.
    pub peek: f64,
    /// How much smaller each card further back in the stack is, as a fraction.
    pub stack_shrink: f64,
    /// How many cards each side of the stack shows; one deeper fades away.
    pub stack_depth: f64,
    /// How much a hovered card grows, as a fraction.
    pub hover: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            card: (0.56, 0.62),
            drop: 0.03,
            roundness: 0.022,
            peek: 0.05,
            stack_shrink: 0.07,
            stack_depth: 3.0,
            hover: 0.02,
        }
    }
}

impl Metrics {
    /// A card's scale `depth` places back in the stack.
    fn shrink(&self, depth: f64) -> f64 {
        1.0 - self.stack_shrink * depth.min(self.stack_depth + 1.0)
    }

    /// How opaque a card `depth` places back in the stack is.
    fn visibility(&self, depth: f64) -> f64 {
        (self.stack_depth + 1.0 - depth).clamp(0.0, 1.0)
    }

    fn is_stacked(&self, depth: usize) -> bool {
        depth as f64 <= self.stack_depth
    }
}

/// A card as drawn: where, how opaque, how round and how deep in the stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub rect: Rectangle<f64, Logical>,
    pub alpha: f64,
    pub radius: f64,
    /// Places behind the top card, negative to its left; 0 is the top card.
    pub depth: f64,
}

/// Lays out one slot per entry, in entry order.
pub fn arrange<K: Copy + Eq>(
    canvas: Canvas,
    metrics: &Metrics,
    strip: &Strip<K>,
    aspect: impl Fn(K) -> f64,
    out: &mut Vec<Slot>,
) {
    out.clear();
    let entries = strip.entries();
    let Some(last) = entries.len().checked_sub(1) else {
        return;
    };
    let unit = f64::from(canvas.output.size.h);
    let usable = canvas.usable.to_f64();
    let bounds = Size::from((
        usable.size.w * metrics.card.0,
        usable.size.h * metrics.card.1,
    ));
    let natural = |index: usize| fit(bounds, aspect(entries[index].key));
    let position = strip.position();

    out.extend(entries.iter().enumerate().map(|(index, entry)| {
        let depth = index as f64 - position;
        let scale =
            entry.presence() * metrics.shrink(depth.abs()) * (1.0 + metrics.hover * entry.lift());
        Slot {
            rect: Rectangle::new(Point::default(), natural(index).upscale(scale)),
            alpha: entry.presence() * metrics.visibility(depth.abs()),
            radius: unit * metrics.roundness * scale,
            depth,
        }
    }));

    // Between two selections each card is part way between where it rests
    // for one and where it rests for the other.
    let below = (position.max(0.0).floor() as usize).min(last);
    let above = (below + 1).min(last);
    let blend = (position - below as f64).clamp(0.0, 1.0);
    for (front, weight) in [(below, 1.0 - blend), (above, blend)] {
        let width = |index: usize| {
            natural(index).w
                * entries[index].presence()
                * metrics.shrink(index.abs_diff(front) as f64)
        };
        let peek = |index: usize| {
            let stacked = metrics.is_stacked(index.abs_diff(front));
            unit * metrics.peek * entries[index].presence() * f64::from(u8::from(stacked))
        };
        chain(entries.len(), front, width, peek, |index, x| {
            out[index].rect.loc.x += x * weight;
        });
    }

    let centre = Point::<f64, Logical>::from((
        usable.loc.x + usable.size.w / 2.0,
        usable.loc.y + usable.size.h / 2.0 + unit * metrics.drop,
    ));
    for slot in out.iter_mut() {
        slot.rect.loc =
            centre + Point::from((slot.rect.loc.x, 0.0)) - slot.rect.size.downscale(2.0).to_point();
    }
}

/// Every card's middle with `front` on top and centred on 0. Out from it the
/// cards are chained: each peeks `peek` past the cards in front of it on the
/// outer side, and never out on the inner side, however wide it is.
fn chain(
    len: usize,
    front: usize,
    width: impl Fn(usize) -> f64,
    peek: impl Fn(usize) -> f64,
    mut place: impl FnMut(usize, f64),
) {
    let half = width(front) / 2.0;
    place(front, 0.0);

    let (mut left, mut right) = (-half, half);
    for index in (0..front).rev() {
        let width = width(index);
        left = left.min(right - width) - peek(index);
        right = left + width;
        place(index, left + width / 2.0);
    }

    let (mut left, mut right) = (-half, half);
    for index in front + 1..len {
        let width = width(index);
        right = right.max(left + width) + peek(index);
        left = right - width;
        place(index, right - width / 2.0);
    }
}

/// The largest size of a window `aspect` wide over high that fits `bounds`.
fn fit(bounds: Size<f64, Logical>, aspect: f64) -> Size<f64, Logical> {
    let aspect = aspect.clamp(ASPECTS.0, ASPECTS.1);
    let width = bounds.w.min(bounds.h * aspect);
    Size::from((width, width / aspect))
}

/// Slot indices from the top card back, the order they are drawn and hit in.
pub fn front_to_back(slots: &[Slot], out: &mut Vec<usize>) {
    out.clear();
    out.extend(0..slots.len());
    out.sort_by(|&a, &b| slots[a].depth.abs().total_cmp(&slots[b].depth.abs()));
}

/// The frontmost card under `at`; `order` is [`front_to_back`]'s.
pub fn slot_at(slots: &[Slot], order: &[usize], at: Point<f64, Logical>) -> Option<usize> {
    order.iter().copied().find(|&index| {
        let slot = &slots[index];
        slot.alpha > 0.5 && slot.rect.contains(at)
    })
}

/// `rect` stepped back towards `point` by `factor`.
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

    const WIDE: f64 = 16.0 / 10.0;

    fn canvas() -> Canvas {
        Canvas::whole(Rectangle::new((0, 0).into(), (1920, 1080).into()))
    }

    fn laid_out(keys: &[u32], direction: Direction, aspect: impl Fn(u32) -> f64) -> Vec<Slot> {
        let mut strip = Strip::default();
        strip.reset(keys.iter().copied(), direction);
        let mut out = Vec::new();
        arrange(canvas(), &Metrics::default(), &strip, aspect, &mut out);
        out
    }

    fn right(rect: Rectangle<f64, Logical>) -> f64 {
        rect.loc.x + rect.size.w
    }

    #[test]
    fn the_selected_card_is_centred_and_largest() {
        let slots = laid_out(&[1, 2, 3], Direction::Forward, |_| WIDE);
        let front = slots[1].rect;
        assert!((spacecontrol::scene::centre(front).x - 960.0).abs() < 1e-6);
        assert!(slots.iter().all(|slot| slot.rect.size.h <= front.size.h));
        assert_eq!(slots[1].depth, 0.0);
    }

    #[test]
    fn earlier_cards_stack_left_and_later_ones_right() {
        let slots = laid_out(&[1, 2, 3, 4, 5], Direction::Forward, |_| WIDE);
        let peek = 1080.0 * Metrics::default().peek;
        let (left, front, next, after) =
            (slots[0].rect, slots[1].rect, slots[2].rect, slots[3].rect);

        assert!((front.loc.x - left.loc.x - peek).abs() < 1e-6);
        assert!(right(left) <= right(front));
        assert!((right(next) - right(front) - peek).abs() < 1e-6);
        assert!((right(after) - right(next) - peek).abs() < 1e-6);
        assert!(next.loc.x >= front.loc.x && after.size.h < next.size.h);
    }

    #[test]
    fn a_wide_card_behind_never_peeks_out_on_the_inner_side() {
        let slots = laid_out(&[1, 2, 3], Direction::Forward, |key| match key {
            2 => 0.6,
            _ => 2.2,
        });
        let front = slots[1].rect;
        assert!(right(slots[0].rect) <= right(front) + 1e-6);
        assert!(slots[2].rect.loc.x >= front.loc.x - 1e-6);
    }

    #[test]
    fn cards_past_the_stack_depth_fade_away() {
        let keys: Vec<_> = (0..12).collect();
        let slots = laid_out(&keys, Direction::Forward, |_| WIDE);
        assert_eq!(slots[4].alpha, 1.0);
        assert_eq!(slots[6].alpha, 0.0);
        assert!(slots.iter().all(|slot| slot.rect.size.w > 0.0));
    }

    #[test]
    fn the_top_card_is_hit_over_the_stack() {
        let slots = laid_out(&[1, 2, 3], Direction::Backward, |_| WIDE);
        let mut order = Vec::new();
        front_to_back(&slots, &mut order);
        assert_eq!(order[0], 2);

        let front = spacecontrol::scene::centre(slots[2].rect);
        assert_eq!(slot_at(&slots, &order, front), Some(2));
        let peek = Point::from((slots[0].rect.loc.x + 1.0, front.y));
        assert_eq!(slot_at(&slots, &order, peek), Some(0));
        assert_eq!(slot_at(&slots, &order, (0.0, 0.0).into()), None);
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

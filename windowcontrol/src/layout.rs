//! Where the strip and every thumbnail in it sit, for one output at one moment.
//!
//! The row is a carousel: thumbnails grow as they near the centred position,
//! so the selected window is the large one in the middle, and the row slides
//! under the strip rather than the strip resizing around it.

use smithay::utils::{Logical, Point, Rectangle};
use spacecontrol::scene::Canvas;

use crate::strip::Strip;

/// Proportions of the output height, so the strip looks the same on every
/// panel.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// A thumbnail's height away from the centre.
    pub thumb: f64,
    /// How much bigger the centred thumbnail is, as a fraction.
    pub grow: f64,
    pub gap: f64,
    /// Inside the strip, around the row.
    pub padding: f64,
    /// Room above the row for a title.
    pub title: f64,
    /// Between the strip and the bottom of the usable area.
    pub margin: f64,
    /// How far a hovered thumbnail rises.
    pub lift: f64,
    /// How much a hovered thumbnail grows, as a fraction.
    pub hover: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            thumb: 0.11,
            grow: 0.45,
            gap: 0.016,
            padding: 0.018,
            title: 0.026,
            margin: 0.022,
            lift: 0.012,
            hover: 0.04,
        }
    }
}

/// A thumbnail as drawn, and how opaque.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub rect: Rectangle<f64, Logical>,
    pub alpha: f64,
}

/// The strip's rectangle. `reveal` is the raw rise, so an overshoot lifts it
/// past its resting place; at 0 it sits just below the usable area.
pub fn panel(canvas: Canvas, metrics: &Metrics, reveal: f64) -> Rectangle<f64, Logical> {
    let unit = f64::from(canvas.output.size.h);
    let usable = canvas.usable.to_f64();
    let height =
        unit * (metrics.thumb * (1.0 + metrics.grow) + metrics.title + 2.0 * metrics.padding);
    let width = (usable.size.w - 2.0 * unit * metrics.margin).max(height);
    let bottom = usable.loc.y + usable.size.h;
    let resting = bottom - unit * metrics.margin - height;
    let y = bottom + (resting - bottom) * reveal;
    Rectangle::new(
        (usable.loc.x + (usable.size.w - width) / 2.0, y).into(),
        (width, height).into(),
    )
}

/// Lays the row out inside `panel`, one slot per entry. `aspect` is a window's
/// width over its height.
pub fn slots<K: Copy + Eq>(
    canvas: Canvas,
    metrics: &Metrics,
    panel: Rectangle<f64, Logical>,
    strip: &Strip<K>,
    aspect: impl Fn(K) -> f64,
    out: &mut Vec<Slot>,
) {
    out.clear();
    let unit = f64::from(canvas.output.size.h);
    let gap = unit * metrics.gap;
    let position = strip.position();
    let row_middle = panel.loc.y + panel.size.h
        - unit * metrics.padding
        - unit * metrics.thumb * (1.0 + metrics.grow) / 2.0;

    let mut x = 0.0;
    let mut centres = (0.0, 0.0);
    for (index, entry) in strip.entries().iter().enumerate() {
        let presence = entry.presence();
        let closeness = (1.0 - (index as f64 - position).abs()).max(0.0);
        let height = unit * metrics.thumb * (1.0 + metrics.grow * closeness);
        let width = height * aspect(entry.key).clamp(0.5, 2.5);
        let advance = (width + gap) * presence;
        let centre = x + advance / 2.0;
        if index == position.floor() as usize {
            centres.0 = centre;
        }
        if index == position.ceil() as usize {
            centres.1 = centre;
        }
        x += advance;

        let lift = entry.lift();
        let size = (width, height).into();
        let rect = Rectangle::new(
            Point::from((
                centre - width / 2.0,
                row_middle - height / 2.0 - unit * metrics.lift * lift,
            )),
            size,
        );
        out.push(Slot {
            rect: shrink(rect, presence * (1.0 + metrics.hover * lift)),
            alpha: presence,
        });
    }

    let anchor = centres.0 + (centres.1 - centres.0) * position.fract();
    let shift = panel.loc.x + panel.size.w / 2.0 - anchor;
    for slot in out.iter_mut() {
        slot.rect.loc.x += shift;
    }
}

/// The topmost slot under `at`, if it is inside the strip.
pub fn slot_at(
    panel: Rectangle<f64, Logical>,
    slots: &[Slot],
    at: Point<f64, Logical>,
) -> Option<usize> {
    if !panel.contains(at) {
        return None;
    }
    slots
        .iter()
        .rposition(|slot| slot.alpha > 0.5 && slot.rect.contains(at))
}

fn shrink(rect: Rectangle<f64, Logical>, factor: f64) -> Rectangle<f64, Logical> {
    spacecontrol::scene::lift(rect, factor - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strip::Direction;

    fn canvas() -> Canvas {
        Canvas::whole(Rectangle::new((0, 0).into(), (1920, 1080).into()))
    }

    fn laid_out(keys: &[u32], direction: Direction) -> (Rectangle<f64, Logical>, Vec<Slot>) {
        let mut strip = Strip::default();
        strip.reset(keys.iter().copied(), direction);
        let metrics = Metrics::default();
        let panel = panel(canvas(), &metrics, 1.0);
        let mut out = Vec::new();
        slots(canvas(), &metrics, panel, &strip, |_| 16.0 / 10.0, &mut out);
        (panel, out)
    }

    #[test]
    fn hidden_strip_sits_below_the_usable_area() {
        let panel = panel(canvas(), &Metrics::default(), 0.0);
        assert_eq!(panel.loc.y, 1080.0);
    }

    #[test]
    fn risen_strip_rests_above_the_bottom_edge() {
        let panel = panel(canvas(), &Metrics::default(), 1.0);
        assert!(panel.loc.y + panel.size.h < 1080.0);
        assert!(panel.loc.y > 540.0);
    }

    #[test]
    fn the_selected_thumbnail_is_centred_and_largest() {
        let (panel, slots) = laid_out(&[1, 2, 3], Direction::Forward);
        let selected = slots[1].rect;
        let centre = selected.loc.x + selected.size.w / 2.0;
        assert!((centre - (panel.loc.x + panel.size.w / 2.0)).abs() < 1e-6);
        assert!(slots.iter().all(|slot| slot.rect.size.h <= selected.size.h));
    }

    #[test]
    fn thumbnails_sit_inside_the_strip_vertically() {
        let (panel, slots) = laid_out(&[1, 2, 3], Direction::Forward);
        for slot in slots {
            assert!(slot.rect.loc.y >= panel.loc.y);
            assert!(slot.rect.loc.y + slot.rect.size.h <= panel.loc.y + panel.size.h);
        }
    }

    #[test]
    fn a_click_finds_the_thumbnail_under_it() {
        let (panel, slots) = laid_out(&[1, 2, 3], Direction::Forward);
        let centre = spacecontrol::scene::centre(slots[2].rect);
        assert_eq!(slot_at(panel, &slots, centre), Some(2));
        assert_eq!(slot_at(panel, &slots, (0.0, 0.0).into()), None);
    }
}

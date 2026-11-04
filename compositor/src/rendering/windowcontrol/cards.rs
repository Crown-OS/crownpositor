//! The stack of cards. Each is its window on a dark base with a soft shadow
//! and its app icon over it; the top card's title sits beside its icon.
//!
//! The top card is its window lifted off the desktop: it flies out of the
//! window's place as the stack comes up and back onto it as the stack goes,
//! while the rest of the stack grows in and fades away around it.

use std::borrow::Cow;

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer, element::Id},
    utils::{Logical, Physical, Point, Rectangle},
};

use spacecontrol::scene;
use windowcontrol::{Slot, layout, motion::ARRIVAL_SCALE};

use crate::{
    rendering::{
        blur::{SHADOW_TAIL, ShadowPiece},
        decorate::{Shadow, TileDecorator},
        painter::{Backing, Painter, Thumbnail},
    },
    shell::{
        monitor::Monitor,
        tile::Tile,
        windowcontrol::{ICON_SIZE, WindowControl},
    },
};

const TITLE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const TITLE_CHARS: usize = 48;
/// Premultiplied: what a see-through window is laid on.
const BASE: [f32; 4] = [0.05, 0.05, 0.06, 0.9];
/// Premultiplied.
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.4];
/// Logical pixels.
const SHADOW_BLUR: f64 = 30.0;
const SHADOW_DROP: f64 = 14.0;
/// Physical pixels: enough to anti-alias the base's edge and no more.
const BASE_SOFTNESS: f32 = 0.5;
/// Between a card's top edge and the middle of its icon.
const ICON_RISE: f64 = ICON_SIZE / 2.0 + 14.0;
/// Between a stacked card's outer edge and its icon.
const ICON_INSET: f64 = 6.0;
const TITLE_GAP: f64 = 10.0;
/// Below this nothing is worth drawing.
const VISIBLE: f64 = 0.01;

/// One card as drawn this frame.
struct Card<'a> {
    tile: &'a Tile,
    rect: Rectangle<f64, Logical>,
    radius: f64,
    alpha: f64,
    /// How much of the stack's furniture shows: the icon, and on the top
    /// card the title.
    chrome: f64,
    focus: f64,
    depth: f64,
}

impl<R, D> Painter<'_, '_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    pub(super) fn cards(&mut self, monitor: &Monitor) {
        let control = monitor.window_control();
        let switcher = control.switcher();
        let shown = switcher.eased();
        let lifted = switcher.progress();
        let arrival = ARRIVAL_SCALE + (1.0 - ARRIVAL_SCALE) * shown;
        let entries = switcher.strip().entries();
        let workspace = monitor.active();

        for &index in control.order() {
            let (Some(entry), Some(slot)) = (entries.get(index), control.slots().get(index)) else {
                continue;
            };
            let Some(tile) = workspace.tile(entry.key) else {
                continue;
            };
            let focus = entry.focus();
            let stacked = layout::recede(slot.rect, control.centre(), arrival);
            let flight = 1.0 - focus * (1.0 - lifted);
            let card = Card {
                tile,
                rect: scene::between(tile.render_rect(), stacked, flight),
                radius: self.flown_radius(slot, arrival, flight),
                alpha: slot.alpha * (1.0 - (1.0 - focus) * (1.0 - shown)),
                chrome: slot.alpha * shown,
                focus,
                depth: slot.depth,
            };
            if card.alpha > VISIBLE {
                self.stack_card(control, &card);
            }
        }
    }

    /// Physical pixels: the window's own radius on the desktop, the card's in
    /// the stack.
    fn flown_radius(&self, slot: &Slot, arrival: f64, flight: f64) -> f64 {
        let window = f64::from(self.radius);
        let card = slot.radius * arrival * self.scale.y;
        window + (card - window) * flight
    }

    fn stack_card(&mut self, control: &WindowControl, card: &Card<'_>) {
        if card.chrome > VISIBLE {
            self.card_chrome(control, card);
        }
        self.thumbnail(Thumbnail {
            tile: card.tile,
            rect: card.rect,
            clip: None,
            alpha: card.alpha as f32,
            backing: Backing::Bare,
            radius: Some(card.radius as f32),
        });
        let Some(ids) = control.card_ids(card.tile.id()) else {
            return;
        };
        let shape = self.physical(card.rect);
        let radius = card.radius as f32;
        let alpha = card.alpha as f32;
        let base = soft_rect(shape, Rectangle::default(), radius, BASE_SOFTNESS, BASE);
        self.card_shadow(ids.base.clone(), base, alpha);

        let dropped = self.physical(Rectangle::new(
            card.rect.loc + Point::from((0.0, SHADOW_DROP)),
            card.rect.size,
        ));
        let blur = (SHADOW_BLUR * self.scale.y) as f32;
        self.card_shadow(
            ids.shadow.clone(),
            soft_rect(dropped, shape, radius, blur, SHADOW),
            alpha,
        );
    }

    /// The icon over the card: at the stacked card's outer edge, moving to
    /// the middle beside the title as the card comes to the top.
    fn card_chrome(&mut self, control: &WindowControl, card: &Card<'_>) {
        let rect = card.rect;
        let y = rect.loc.y - ICON_RISE;
        let edge = ICON_INSET + ICON_SIZE / 2.0;
        let stacked = match card.depth < 0.0 {
            true => rect.loc.x + edge,
            false => rect.loc.x + rect.size.w - edge,
        };

        let title = shortened(card.tile.title());
        let width = match card.focus > VISIBLE {
            true => f64::from(self.style.text.measure(&title, self.scale.x, TITLE, true)),
            false => 0.0,
        };
        let middle = scene::centre(rect).x;
        let group = ICON_SIZE + TITLE_GAP + width;
        let icon = Point::<f64, Logical>::from((
            stacked + (middle - group / 2.0 + ICON_SIZE / 2.0 - stacked) * card.focus,
            y,
        ));
        let has_icon = match control.icon(card.tile.id()) {
            Some((buffer, size)) => {
                self.sprite(buffer, size, icon, card.chrome as f32);
                true
            }
            None => false,
        };

        let alpha = card.chrome * card.focus;
        if width > 0.0 && alpha > VISIBLE {
            let x = match has_icon {
                true => icon.x + ICON_SIZE / 2.0 + TITLE_GAP + width / 2.0,
                false => middle,
            };
            self.label(&title, (x, y).into(), TITLE, alpha as f32);
        }
    }

    fn card_shadow(&mut self, id: Id, piece: ShadowPiece, alpha: f32) {
        let shadow = Shadow {
            id,
            commit: self.glass_commit(),
            piece,
            alpha,
        };
        if let Some(element) = self.decorator.shadow(self.renderer, shadow) {
            self.push(element);
        }
    }
}

/// `shape` rounded by `radius` and blurred by `sigma`, all physical pixels,
/// and left out under `hole`.
fn soft_rect(
    shape: Rectangle<i32, Physical>,
    hole: Rectangle<i32, Physical>,
    radius: f32,
    sigma: f32,
    color: [f32; 4],
) -> ShadowPiece {
    let tail = (sigma * SHADOW_TAIL).ceil() as i32;
    let margin = Point::from((tail, tail));
    ShadowPiece {
        geometry: Rectangle::from_extremities(
            shape.loc - margin,
            shape.loc + shape.size.to_point() + margin,
        ),
        shape,
        hole,
        radius,
        sigma,
        color,
    }
}

/// A title short enough to sit over its card.
fn shortened(title: &str) -> Cow<'_, str> {
    match title.char_indices().nth(TITLE_CHARS) {
        Some((end, _)) => Cow::Owned(format!("{}…", title[..end].trim_end())),
        None => Cow::Borrowed(title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_title_is_left_alone() {
        assert!(matches!(shortened("kitty"), Cow::Borrowed("kitty")));
    }

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis() {
        let long = "a".repeat(TITLE_CHARS + 10);
        let short = shortened(&long);
        assert_eq!(short.chars().count(), TITLE_CHARS + 1);
        assert!(short.ends_with('…'));
    }
}

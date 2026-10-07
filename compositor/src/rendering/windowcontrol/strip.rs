//! The glass strip and the thumbnails in it.

use std::borrow::Cow;

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer},
    utils::{Logical, Point, Rectangle},
};

use spacecontrol::scene;
use windowcontrol::Slot;

use crate::{
    rendering::{
        blur::{GlassKind, SHADOW_TAIL, ShadowPiece},
        decorate::{Shadow, TileDecorator},
        decoration::window::Border,
        painter::{Backing, Painter, Thumbnail},
    },
    shell::{monitor::Monitor, tile::Tile, windowcontrol::WindowControl},
};

const RING: [f32; 4] = [1.0, 1.0, 1.0, 0.9];
const TITLE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// Premultiplied.
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.32];
/// Logical pixels.
const SHADOW_BLUR: f64 = 28.0;
const SHADOW_DROP: f64 = 10.0;
/// Between a thumbnail's top edge and the middle of its title.
const TITLE_RISE: f64 = 14.0;
const TITLE_CHARS: usize = 48;
/// The strip's corner radius, as a fraction of its height.
const ROUNDNESS: f64 = 0.22;

impl<R, D> Painter<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    pub(super) fn strip(&mut self, monitor: &Monitor) {
        let control = monitor.window_control();
        let reveal = control.switcher().eased() as f32;
        if reveal <= 0.0 {
            return;
        }
        let shown = || {
            let workspace = monitor.active();
            control
                .switcher()
                .strip()
                .entries()
                .iter()
                .zip(control.slots())
                .enumerate()
                .filter(|(_, (entry, _))| Some(entry.key) != control.switcher().committed())
                .filter_map(move |(index, (entry, slot))| {
                    Some((index, entry.lift(), workspace.tile(entry.key)?, *slot))
                })
        };

        let selected = control
            .switcher()
            .is_open()
            .then(|| control.switcher().strip().selected());
        for (index, lift, tile, slot) in shown() {
            let emphasis = if Some(index) == selected { 1.0 } else { lift };
            self.title(tile, slot, (emphasis * slot.alpha) as f32 * reveal);
        }
        if let Some((_, _, tile, slot)) = shown().find(|(index, ..)| Some(*index) == selected) {
            self.ring(control, tile, slot, reveal);
        }
        let clip = self.physical(control.panel());
        for (_, _, tile, slot) in shown() {
            self.thumbnail(Thumbnail {
                window: tile.window(),
                rect: slot.rect,
                clip: Some(clip),
                alpha: slot.alpha as f32 * reveal,
                backing: Backing::Bare,
            });
        }
        self.panel(control, reveal);
    }

    fn title(&mut self, tile: &Tile, slot: Slot, alpha: f32) {
        if alpha <= 0.01 {
            return;
        }
        let centre = Point::from((scene::centre(slot.rect).x, slot.rect.loc.y - TITLE_RISE));
        self.label(&shortened(tile.title()), centre, TITLE, alpha);
    }

    fn ring(&mut self, control: &WindowControl, tile: &Tile, slot: Slot, alpha: f32) {
        let width = tile.window().geometry().size.w;
        if width <= 0 {
            return;
        }
        let border = Border {
            id: control.ids().ring.clone(),
            commit: self.glass_commit(),
            window: self.physical(slot.rect),
            thickness: self.scale.x as f32 * 2.5,
            radius: self.radius * (slot.rect.size.w / f64::from(width)) as f32,
            color: RING,
            alpha: alpha * slot.alpha as f32,
        };
        if let Some(ring) = self.decorator.border(self.renderer, border) {
            self.push(ring);
        }
    }

    /// The strip's shadow, then its glass: the shadow is cut away under the
    /// glass, so drawing it in front keeps it out of what the glass blurs.
    fn panel(&mut self, control: &WindowControl, alpha: f32) {
        let panel = control.panel();
        let geometry = self.physical(panel);
        if geometry.is_empty() {
            return;
        }
        let radius = (panel.size.h * ROUNDNESS * self.scale.y) as f32;

        let sigma = (SHADOW_BLUR * self.scale.y) as f32;
        let tail = (sigma * SHADOW_TAIL).ceil() as i32;
        let shape = self.physical(Rectangle::new(
            panel.loc + Point::<f64, Logical>::from((0.0, SHADOW_DROP)),
            panel.size,
        ));
        let piece = ShadowPiece {
            geometry: Rectangle::from_extremities(
                shape.loc - Point::from((tail, tail)),
                shape.loc + shape.size.to_point() + Point::from((tail, tail)),
            ),
            shape,
            hole: geometry,
            radius,
            sigma,
            color: SHADOW,
        };
        let shadow = Shadow {
            id: control.ids().shadow.clone(),
            commit: self.glass_commit(),
            piece,
            alpha,
        };
        if let Some(shadow) = self.decorator.shadow(self.renderer, shadow) {
            self.push(shadow);
        }

        self.glass(
            control.ids().panel.clone(),
            GlassKind::Panel,
            geometry,
            radius,
            1.0,
            alpha,
        );
    }
}

/// A title short enough to sit over its thumbnail.
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

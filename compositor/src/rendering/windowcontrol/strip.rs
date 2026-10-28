//! The glass panel and the cards on it.

use std::borrow::Cow;

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer},
    utils::{Logical, Point, Rectangle},
};

use spacecontrol::scene;

use crate::{
    rendering::{
        blur::{GlassKind, SHADOW_TAIL, ShadowPiece},
        decorate::{Shadow, TileDecorator},
        painter::{Backing, Painter, Thumbnail},
    },
    shell::{monitor::Monitor, windowcontrol::WindowControl},
};

const TITLE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// Premultiplied.
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.32];
/// Logical pixels.
const SHADOW_BLUR: f64 = 28.0;
const SHADOW_DROP: f64 = 10.0;
/// Between the panel's top edge and the middle of the title over it.
const TITLE_RISE: f64 = 18.0;
const TITLE_CHARS: usize = 48;
/// Below this a title is not worth rasterising.
const TITLE_VISIBLE: f64 = 0.01;

impl<R, D> Painter<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    pub(super) fn strip(&mut self, monitor: &Monitor) {
        let control = monitor.window_control();
        let reveal = control.switcher().eased();
        if reveal <= 0.0 {
            return;
        }
        let workspace = monitor.active();
        let panel = control.panel();
        let cards = || {
            control
                .switcher()
                .strip()
                .entries()
                .iter()
                .zip(control.slots())
                .rev()
                .filter_map(|(entry, slot)| Some((entry, workspace.tile(entry.key)?, *slot)))
        };

        for (entry, tile, slot) in cards() {
            let alpha = entry.focus() * slot.alpha * reveal;
            if alpha > TITLE_VISIBLE {
                let centre =
                    Point::from((scene::centre(slot.rect).x, panel.rect.loc.y - TITLE_RISE));
                self.label(&shortened(tile.title()), centre, TITLE, alpha as f32);
            }
        }

        let clip = self.physical(panel.row);
        for (_, tile, slot) in cards() {
            self.thumbnail(Thumbnail {
                window: tile.window(),
                rect: slot.rect,
                clip: Some(clip),
                alpha: (slot.alpha * reveal) as f32,
                backing: Backing::Bare,
                radius: Some((slot.radius * self.scale.y) as f32),
            });
        }
        self.panel(control, reveal as f32);
    }

    /// The panel's shadow, then its glass: the shadow is cut away under the
    /// glass, so drawing it in front keeps it out of what the glass blurs.
    fn panel(&mut self, control: &WindowControl, alpha: f32) {
        let panel = control.panel();
        let geometry = self.physical(panel.rect);
        if geometry.is_empty() {
            return;
        }
        let radius = (panel.radius * self.scale.y) as f32;

        let sigma = (SHADOW_BLUR * self.scale.y) as f32;
        let tail = (sigma * SHADOW_TAIL).ceil() as i32;
        let shape = self.physical(Rectangle::new(
            panel.rect.loc + Point::<f64, Logical>::from((0.0, SHADOW_DROP)),
            panel.rect.size,
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

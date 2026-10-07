//! The workspace bar: a preview of every workspace, and the tile that adds
//! one.
//!
//! A preview is a small copy of a workspace, so it is made of what a workspace
//! is made of — the wallpaper blurred behind its windows — rather than a flat
//! card.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{Kind, solid::SolidColorRenderElement},
        utils::CommitCounter,
    },
    utils::{Logical, Point, Rectangle},
};

use spacecontrol::scene;

use crate::{
    rendering::{
        blur::GlassKind,
        decorate::TileDecorator,
        decoration::window::Border,
        element::CrownElement,
        painter::{Backing, Painter, Thumbnail},
    },
    shell::{monitor::Monitor, overview::TileIds},
};

/// The ring around the workspace on screen.
const ACTIVE_RING: [f32; 4] = [0.20, 0.51, 0.98, 1.0];
const GLYPH: [f32; 4] = [1.0, 1.0, 1.0, 0.85];
const LABEL: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// One rounded sheet of glass in the bar.
struct Card {
    rect: Rectangle<f64, Logical>,
    /// Physical pixels.
    radius: f32,
    /// How far what the card stands for is shrunk into it, which its rim and
    /// its blur shrink with.
    shrink: f64,
    alpha: f32,
    ring: Option<[f32; 4]>,
}

impl<R, D> Painter<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    pub(super) fn bar(&mut self, monitor: &Monitor) {
        let space = monitor.spacecontrol();
        let reveal = space.overview().bar();
        if reveal <= 0.0 {
            return;
        }
        // The bar enters from below the bottom edge rather than fading in on
        // the spot.
        let climb = scene::climb(space.canvas(), space.metrics(), reveal);
        // The ring follows the viewport, so a swipe across the overview is
        // seen to change workspace as it crosses.
        let shown = monitor.switch().position().round().max(0.0) as usize;
        let lifted = space.lifted_preview();

        let rest = (0..space.bar().len()).filter(|index| Some(*index) != lifted);
        for index in lifted.into_iter().chain(rest) {
            self.preview(monitor, index, climb, reveal, index == shown);
        }
        self.add_tile(monitor, climb, reveal);
    }

    fn preview(&mut self, monitor: &Monitor, index: usize, climb: f64, reveal: f64, active: bool) {
        let space = monitor.spacecontrol();
        let (Some(slot), Some(workspace), Some(ids)) = (
            space.preview_slot(index),
            monitor.workspaces().get(index),
            space.ids().preview(index),
        ) else {
            return;
        };
        let rect = scene::shown(
            slot.thumb,
            climb,
            space.workspace_lift(index),
            space.metrics(),
        );

        let close = space.close_visibility(index);
        if close > 0.0 {
            self.close_button(rect, ids, (reveal * close) as f32);
        }

        let area = monitor.usable();
        let clip = self.physical(rect);
        for tile in workspace.stacking_order() {
            self.thumbnail(Thumbnail {
                window: tile.window(),
                rect: scene::inside(tile.target(), area, rect),
                clip: Some(clip),
                alpha: reveal as f32,
                backing: Backing::Bare,
            });
        }

        // Concentric with the windows inside: their corners, pushed out by
        // the gap they keep from the edge.
        let shrink = rect.size.w / f64::from(area.size.w.max(1));
        let inset = (f64::from(monitor.gaps().outer) * self.scale.x) as f32;
        self.card(
            ids,
            Card {
                rect,
                radius: (self.radius + inset) * shrink as f32,
                shrink,
                alpha: reveal as f32,
                ring: active.then_some(ACTIVE_RING),
            },
        );

        let label = scene::centre(slot.label) + Point::from((0.0, climb));
        self.label(&(index + 1).to_string(), label, LABEL, reveal as f32);
    }

    /// The '×' that removes a workspace: a glyph on a disc of glass.
    fn close_button(&mut self, preview: Rectangle<f64, Logical>, ids: &TileIds, alpha: f32) {
        let button = scene::close_button(preview);
        self.label("×", scene::centre(button), LABEL, alpha);
        let geometry = self.physical(button);
        self.glass(
            ids.glyph[0].clone(),
            GlassKind::Window,
            geometry,
            geometry.size.w as f32 / 2.0,
            1.0,
            alpha,
        );
    }

    fn add_tile(&mut self, monitor: &Monitor, climb: f64, reveal: f64) {
        let space = monitor.spacecontrol();
        let Some(thumb) = space.add_tile() else {
            return;
        };
        let ids = &space.ids().add;
        let rect = scene::shown(thumb, climb, space.add_lift(), space.metrics());
        self.plus(ids, rect, reveal as f32);
        self.card(
            ids,
            Card {
                rect,
                radius: self.radius * 0.5,
                shrink: rect.size.w / f64::from(space.canvas().usable.size.w.max(1)),
                alpha: reveal as f32,
                ring: None,
            },
        );
    }

    /// The '+' on the add tile: two bars, crisp at any size.
    fn plus(&mut self, ids: &TileIds, tile: Rectangle<f64, Logical>, alpha: f32) {
        let arm = tile.size.w.min(tile.size.h) * 0.32;
        let stroke = (arm * 0.12).max(2.0);
        let Point { x, y, .. } = scene::centre(tile);
        let bars = [
            Rectangle::new(
                (x - arm / 2.0, y - stroke / 2.0).into(),
                (arm, stroke).into(),
            ),
            Rectangle::new(
                (x - stroke / 2.0, y - arm / 2.0).into(),
                (stroke, arm).into(),
            ),
        ];
        let colour = [GLYPH[0], GLYPH[1], GLYPH[2], GLYPH[3] * alpha];
        for (id, bar) in ids.glyph.iter().zip(bars) {
            let geometry = self.physical(bar);
            if geometry.is_empty() || alpha <= 0.0 {
                continue;
            }
            self.elements
                .push(CrownElement::Solid(SolidColorRenderElement::new(
                    id.clone(),
                    geometry,
                    CommitCounter::default(),
                    colour,
                    Kind::Unspecified,
                )));
        }
    }

    /// A card's ring, then the glass it is made of.
    fn card(&mut self, ids: &TileIds, card: Card) {
        let geometry = self.physical(card.rect);
        if geometry.is_empty() || card.alpha <= 0.0 {
            return;
        }
        if let Some(color) = card.ring {
            let border = Border {
                id: ids.ring.clone(),
                commit: self.glass_commit(),
                window: geometry,
                thickness: self.scale.x as f32 * 2.0,
                radius: card.radius,
                color,
                alpha: card.alpha,
            };
            if let Some(ring) = self.decorator.border(self.renderer, border) {
                self.push(ring);
            }
        }
        self.glass(
            ids.glass.clone(),
            GlassKind::Window,
            geometry,
            card.radius,
            card.shrink,
            card.alpha,
        );
    }
}

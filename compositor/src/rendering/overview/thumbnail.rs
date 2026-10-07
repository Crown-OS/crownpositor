//! Windows drawn at another size: flying between the desktop and the grid,
//! carried by the pointer, or shrunk into their workspace's preview.
//!
//! A thumbnail is the window's own surfaces with a different destination
//! rectangle, so the GPU recomposites textures it already holds — no readback,
//! no copy, and a video keeps playing in its thumbnail.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Wrap,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
    },
    desktop::Window,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
};

use spacecontrol::scene;

use super::Painter;
use crate::{
    rendering::{
        backdrop_elements,
        blur::{self, GlassKind},
        decorate::{Outline, TileDecorator},
        element::CrownElement,
    },
    shell::monitor::Monitor,
};

/// What a thumbnail stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Backing {
    /// The glass the window asked for on the desktop, shrunk with it.
    Glass,
    /// Nothing: a preview's thumbnails sit on the preview's own glass. Their
    /// window is usually in the grid too, and its glass drawn twice would be
    /// one identity in two places on one frame.
    Bare,
}

pub(super) struct Thumbnail<'a> {
    pub window: &'a Window,
    /// Where it is drawn, output-local.
    pub rect: Rectangle<f64, Logical>,
    /// What it is cut to, if anything: a preview's card.
    pub clip: Option<Rectangle<i32, Physical>>,
    pub alpha: f32,
    pub backing: Backing,
}

impl<R, D> Painter<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    /// The window under the pointer, above everything it can be dropped on.
    pub(super) fn carried(&mut self, monitor: &Monitor) {
        let Some((slot, rect)) = monitor.spacecontrol().carrying() else {
            return;
        };
        let Some(tile) = monitor.active().tiles().get(slot) else {
            return;
        };
        self.thumbnail(Thumbnail {
            window: tile.window(),
            rect,
            clip: None,
            alpha: tile.render_alpha(),
            backing: Backing::Glass,
        });
    }

    /// Every workspace with part of its grid on screen, at the offset the
    /// viewport spring has it at — a swipe across the overview slides pages
    /// exactly the way the desktop does.
    ///
    /// Stacking order, topmost first, so windows keep their depth on the way
    /// into the grid.
    pub(super) fn grid(&mut self, monitor: &Monitor) {
        let space = monitor.spacecontrol();
        let progress = space.overview().progress();
        let carried = space.carrying().map(|(slot, _)| slot);
        let stride = monitor.page_stride();

        for (index, page) in monitor.switch().visible(monitor.workspaces().len()) {
            let Some(workspace) = monitor.workspaces().get(index) else {
                continue;
            };
            let offset = Point::from((page * stride, 0.0));
            let active = index == monitor.active_index();

            for tile in workspace.stacking_order() {
                let Some(slot) = workspace.tiles().iter().position(|it| it.id() == tile.id())
                else {
                    continue;
                };
                if active && carried == Some(slot) {
                    continue;
                }
                let Some(cell) = space.grid_rect(index, slot) else {
                    continue;
                };
                // Scaled by progress, so a window under the pointer on the way
                // in grows with everything else instead of jumping.
                let lift = space.metrics().hover * space.window_lift(index, slot) * progress;
                let rect = scene::between(tile.render_rect(), scene::lift(cell, lift), progress);
                self.thumbnail(Thumbnail {
                    window: tile.window(),
                    rect: Rectangle::new(rect.loc + offset, rect.size),
                    clip: None,
                    alpha: tile.render_alpha(),
                    backing: Backing::Glass,
                });
            }
        }
    }

    /// Draws one window at `thumb.rect`, whatever size that is: its surfaces
    /// are placed at the rect's corner and scaled about it, so a thumbnail and
    /// a full-size window differ only in the factor.
    pub(super) fn thumbnail(&mut self, thumb: Thumbnail<'_>) {
        let geometry = thumb.window.geometry();
        if geometry.size.w <= 0 || geometry.size.h <= 0 || thumb.rect.size.is_empty() {
            return;
        }

        let shape = self.physical(thumb.rect);
        let Some(crop) = thumb
            .clip
            .map_or(Some(shape), |clip| shape.intersection(clip))
        else {
            return;
        };
        let shrink = Scale::from((
            thumb.rect.size.w / f64::from(geometry.size.w),
            thumb.rect.size.h / f64::from(geometry.size.h),
        ));
        let radius = self.radius * shrink.x.min(shrink.y) as f32;

        // The geometry's corner lands on the rect's; the client's own origin
        // sits the geometry's offset before it.
        let surface_origin = shape.loc - geometry.loc.to_physical_precise_round(self.scale);
        let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
            thumb
                .window
                .render_elements(self.renderer, surface_origin, self.scale, thumb.alpha);
        for surface in surfaces {
            let scaled = RescaleRenderElement::from_element(surface, shape.loc, shrink);
            let Some(cropped) = CropRenderElement::from_element(scaled, self.scale, crop) else {
                continue;
            };
            if let Some(decorated) =
                self.decorator
                    .decorate(self.renderer, cropped, shape, [radius; 4])
            {
                self.push(decorated);
            }
        }

        // After the surfaces, so it lands directly behind them.
        if thumb.backing == Backing::Glass {
            self.window_glass(thumb.window, thumb.rect, geometry.size, radius, thumb.alpha);
        }
    }

    /// The blur a window asked for, placed behind its thumbnail. A client
    /// states its blur region in its own surface coordinates, so a thumbnail's
    /// is the same placement at the shrink factor times the output's scale.
    fn window_glass(
        &mut self,
        window: &Window,
        rect: Rectangle<f64, Logical>,
        natural: Size<i32, Logical>,
        radius: f32,
        alpha: f32,
    ) {
        let Some(surface) = blur::window_surface(window) else {
            return;
        };
        let scale = self.scale;
        let shrink = Scale::from((
            scale.x * rect.size.w / f64::from(natural.w),
            scale.y * rect.size.h / f64::from(natural.h),
        ));
        let mask = self.physical(rect);
        let offset = window.geometry().loc.to_f64();
        let origin = mask.loc
            - Point::<f64, Physical>::from((offset.x * shrink.x, offset.y * shrink.y))
                .to_i32_round();
        let strength = self
            .decorator
            .blur_strength_for(shrink.x.min(shrink.y) / scale.x);

        let Self {
            elements,
            renderer,
            decorator,
            ..
        } = self;
        backdrop_elements(
            &mut |element| elements.push(CrownElement::Tile(Wrap::from(element))),
            *renderer,
            *decorator,
            &surface,
            origin,
            shrink,
            mask,
            Outline { rect: mask, radius },
            alpha,
            GlassKind::Window,
            strength,
        );
    }
}

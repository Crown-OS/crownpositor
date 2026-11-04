//! Windows drawn at another size: a thumbnail is the window's own surfaces
//! with a different destination rectangle, so the GPU recomposites textures it
//! already holds — no readback, no copy, and a video keeps playing in it.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Wrap,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Scale},
};

use super::Painter;
use crate::{
    rendering::{
        backdrop_elements,
        blur::{self, Glass, GlassKind},
        content_radii,
        decorate::{Outline, TileDecorator},
        element::CrownElement,
        frame::{Frame, client_area},
    },
    shell::tile::Tile,
};

/// What a thumbnail stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::rendering) enum Backing {
    /// The glass the window asked for on the desktop, shrunk with it.
    Glass,
    /// Nothing: a preview's thumbnails sit on the preview's own glass. Their
    /// window is usually in the grid too, and its glass drawn twice would be
    /// one identity in two places on one frame.
    Bare,
}

pub(in crate::rendering) struct Thumbnail<'a> {
    pub tile: &'a Tile,
    /// Where it is drawn, output-local: the whole window, titlebar included.
    pub rect: Rectangle<f64, Logical>,
    /// What it is cut to, if anything: a preview's card.
    pub clip: Option<Rectangle<i32, Physical>>,
    pub alpha: f32,
    pub backing: Backing,
    /// Its corner radius in physical pixels, if not the window's own shrunk
    /// with it: a card concentric with the panel it sits on.
    pub radius: Option<f32>,
}

impl<R, D> Painter<'_, '_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    /// Draws one window at `thumb.rect`, whatever size that is: its frame and
    /// its surfaces are placed at the rect's corner and scaled about it, so a
    /// thumbnail and a full-size window differ only in the factor.
    pub(in crate::rendering) fn thumbnail(&mut self, thumb: Thumbnail<'_>) {
        let tile = thumb.tile;
        let geometry = tile.window().geometry();
        if geometry.size.w <= 0 || geometry.size.h <= 0 || thumb.rect.size.is_empty() {
            return;
        }

        let shape = self.physical(thumb.rect);
        if shape.is_empty() {
            return;
        }
        let Some(crop) = thumb
            .clip
            .map_or(Some(shape), |clip| shape.intersection(clip))
        else {
            return;
        };
        let (titlebar, rim) = (tile.render_titlebar(), tile.render_rim());
        let shrink = Scale::from((
            thumb.rect.size.w / (f64::from(geometry.size.w) + 2.0 * rim),
            thumb.rect.size.h / (f64::from(geometry.size.h) + titlebar + rim),
        ));
        let zoom = shrink.x.min(shrink.y);
        let radius = thumb.radius.unwrap_or(self.radius * zoom as f32);
        let inset = (titlebar * shrink.y * self.scale.y).round() as i32;
        let rim = (rim * zoom * self.scale.x).round() as i32;
        let content = client_area(shape, inset, rim);
        let reveal = tile.titlebar_reveal();
        let frame = (inset > 0).then(|| Frame {
            tile,
            ids: match thumb.backing {
                Backing::Glass => tile.decoration_ids(),
                Backing::Bare => tile.preview_decoration_ids(),
            },
            rect: shape,
            inset,
            radius,
            zoom,
            backing: thumb.backing,
            clip: thumb.clip,
        });

        if let Some(frame) = &frame {
            frame.overlay::<R, D>(
                self.elements,
                self.renderer,
                self.decorator,
                self.style,
                thumb.alpha * reveal,
            );
        }

        let radii = content_radii(radius, rim as f32);
        if let Some(visible) = content.intersection(crop) {
            self.surfaces(tile, content, visible, shrink, radii, thumb.alpha);
        }

        // After the surfaces, so it lands directly behind them, cut to the
        // client's shape and flat inside a frame, as on the desktop.
        if thumb.backing == Backing::Glass {
            let physical = Scale::from((self.scale.x * shrink.x, self.scale.y * shrink.y));
            let glass = self.decorator.glass(physical.x, GlassKind::Window);
            let glass = match frame {
                Some(_) => glass.flat(),
                None => glass,
            };
            let outline = Outline {
                rect: content,
                radius: radii[0],
            };
            self.window_glass(tile, content, physical, outline, glass, thumb.alpha);
        }

        if let Some(frame) = &frame {
            frame.backing::<R, D>(
                self.elements,
                self.renderer,
                self.decorator,
                self.style,
                thumb.alpha * reveal,
            );
        }
    }

    /// The client's surfaces with the geometry's corner on `content`'s; the
    /// client's own origin sits the geometry's offset before it.
    fn surfaces(
        &mut self,
        tile: &Tile,
        content: Rectangle<i32, Physical>,
        visible: Rectangle<i32, Physical>,
        shrink: Scale<f64>,
        radii: [f32; 4],
        alpha: f32,
    ) {
        let window = tile.window();
        let surface_origin =
            content.loc - window.geometry().loc.to_physical_precise_round(self.scale);
        let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
            window.render_elements(self.renderer, surface_origin, self.scale, alpha);
        for surface in surfaces {
            let scaled = RescaleRenderElement::from_element(surface.into(), content.loc, shrink);
            let Some(cropped) = CropRenderElement::from_element(scaled, self.scale, visible) else {
                continue;
            };
            if let Some(decorated) = self
                .decorator
                .decorate(self.renderer, cropped, content, radii)
            {
                self.push(decorated);
            }
        }
    }

    /// The blur a window asked for, placed behind its thumbnail. A client
    /// states its blur region in its own surface coordinates, so a thumbnail's
    /// is the same placement at the shrink factor times the output's scale.
    fn window_glass(
        &mut self,
        tile: &Tile,
        content: Rectangle<i32, Physical>,
        physical: Scale<f64>,
        outline: Outline,
        glass: Glass,
        alpha: f32,
    ) {
        let window = tile.window();
        let Some(surface) = blur::window_surface(window) else {
            return;
        };
        let offset = window.geometry().loc.to_f64();
        let origin = content.loc
            - Point::<f64, Physical>::from((offset.x * physical.x, offset.y * physical.y))
                .to_i32_round();
        let strength = self
            .decorator
            .blur_strength_for(physical.x.min(physical.y) / self.scale.x);

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
            physical,
            content,
            outline,
            alpha,
            glass,
            strength,
        );
    }
}

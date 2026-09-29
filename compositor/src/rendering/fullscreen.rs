//! The fast path for a settled fullscreen window: its surfaces, untouched.
//!
//! No rounding, opacity, glass, frame or panels, so the client's buffer is a
//! candidate for the primary plane and a game's frame reaches the screen with
//! no composition at all.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            Kind,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
        },
    },
    utils::{Physical, Point, Scale},
};

use crate::{
    rendering::{Elements, decorate::TileDecorator, element::CrownElement, popup},
    shell::tile::Tile,
};

pub fn fullscreen_elements<R, D>(
    elements: &mut Elements<R, D>,
    tile: &Tile,
    renderer: &mut R,
    scale: Scale<f64>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let origin: Point<i32, Physical> = tile.render_rect().loc.to_physical_precise_round(scale);
    elements.extend(
        popup::popup_elements(renderer, tile.window(), origin, scale, 1.0)
            .into_iter()
            .map(CrownElement::Surface),
    );
    let surfaces: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
        renderer,
        tile.surface(),
        origin,
        scale,
        1.0,
        Kind::ScanoutCandidate,
    );
    elements.extend(surfaces.into_iter().map(CrownElement::Surface));
}

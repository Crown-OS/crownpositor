//! Popups, drawn whole rather than cropped to the surface that opened them: a
//! menu is free to hang past the edge of its window.
//!
//! A window's popups are not drawn with the window. They form a stratum of
//! their own over every window and every panel, so a menu hanging past its
//! window's edge is never cut off by the window above or the dock below —
//! and the pointer is routed to them first, in the same order.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{Kind, Wrap, utils::RescaleRenderElement},
    },
    desktop::{LayerSurface, PopupManager, Window},
    utils::{Logical, Physical, Point, Scale},
};

use crate::{
    rendering::{
        Elements, Placed,
        decorate::{self, TileDecorator},
        element::CrownElement,
        surface_tree::{SurfaceElement, TransformedSurface, surface_elements},
    },
    shell::{tile::Tile, workspace::Workspace},
};

/// A window's popups, placed from where its own `(0, 0)` lands.
pub fn popup_elements<R>(
    renderer: &mut R,
    window: &Window,
    surface_origin: Point<i32, Physical>,
    scale: Scale<f64>,
    alpha: f32,
) -> Vec<TransformedSurface<R>>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    let Some(toplevel) = window.toplevel() else {
        return Vec::new();
    };
    let geometry_origin = window.geometry().loc;

    PopupManager::popups_for_surface(toplevel.wl_surface())
        .flat_map(|(popup, offset)| {
            let location = surface_origin
                + (geometry_origin + offset - popup.geometry().loc)
                    .to_physical_precise_round(scale);
            surface_elements(
                renderer,
                popup.wl_surface(),
                location,
                scale,
                alpha,
                Kind::Unspecified,
            )
        })
        .map(unscaled)
        .collect()
}

/// One tile's popups, where the tile is drawn this frame.
pub(crate) fn tile_popup_elements<R, D>(
    elements: &mut Elements<R, D>,
    tile: &Tile,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    offset: Point<f64, Logical>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    let origin = Placed::new(tile, offset, scale).surface_origin;
    let popups = popup_elements(renderer, tile.window(), origin, scale, tile.render_alpha());
    elements.extend(
        decorate::unclipped(renderer, decorator, popups, scale)
            .into_iter()
            .map(|popup| CrownElement::Tile(Wrap::from(popup))),
    );
}

/// The popups of every window `workspace` shows, in the windows' own order.
/// A settled fullscreen window is the only window shown, and a window on its
/// way into its icon has let go of its popups.
pub(crate) fn workspace_popup_elements<R, D>(
    elements: &mut Elements<R, D>,
    workspace: &Workspace,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    offset: Point<f64, Logical>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    let settled = workspace.fullscreen().filter(|id| {
        workspace
            .tile(*id)
            .is_some_and(|tile| tile.morph().is_none())
    });
    let shown = workspace
        .stacking_order()
        .filter(|tile| settled.is_none_or(|id| id == tile.id()) && tile.morph().is_none());
    for tile in shown {
        tile_popup_elements(elements, tile, renderer, decorator, scale, offset);
    }
}

/// A layer surface's popups, placed the way smithay's `LayerSurface` places
/// them, for callers that draw the surface tree itself separately.
pub fn layer_popup_elements<R>(
    renderer: &mut R,
    layer: &LayerSurface,
    location: Point<i32, Physical>,
    scale: Scale<f64>,
) -> Vec<TransformedSurface<R>>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    PopupManager::popups_for_surface(layer.wl_surface())
        .flat_map(|(popup, popup_offset)| {
            let offset = (popup_offset - popup.geometry().loc)
                .to_f64()
                .to_physical(scale)
                .to_i32_round();
            surface_elements(
                renderer,
                popup.wl_surface(),
                location + offset,
                scale,
                1.0,
                Kind::Unspecified,
            )
        })
        .map(unscaled)
        .collect()
}

/// Wrapped at scale 1, which draws and damages exactly like the bare element,
/// so a popup can go through the decorator the way a window's surfaces do.
fn unscaled<R>(element: SurfaceElement<R>) -> TransformedSurface<R>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    RescaleRenderElement::from_element(element, Point::default(), 1.0)
}

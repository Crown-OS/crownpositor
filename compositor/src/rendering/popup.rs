use smithay::{
    backend::renderer::{
        ImportAll, Renderer,
        element::{
            Kind,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
        },
    },
    desktop::{LayerSurface, PopupManager, Window},
    utils::{Physical, Point, Scale},
};

/// A window's popups, drawn whole rather than cropped to the window: a menu is
/// free to hang past the edge of the window that opened it.
pub fn popup_elements<R>(
    renderer: &mut R,
    window: &Window,
    surface_origin: Point<i32, Physical>,
    scale: Scale<f64>,
    alpha: f32,
) -> Vec<WaylandSurfaceRenderElement<R>>
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
            render_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                location,
                scale,
                alpha,
                Kind::Unspecified,
            )
        })
        .collect()
}

/// A layer surface's popups, placed the way smithay's `LayerSurface` places
/// them, for callers that draw the surface tree itself separately.
pub fn layer_popup_elements<R>(
    renderer: &mut R,
    layer: &LayerSurface,
    location: Point<i32, Physical>,
    scale: Scale<f64>,
) -> Vec<WaylandSurfaceRenderElement<R>>
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
            render_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                location + offset,
                scale,
                1.0,
                Kind::Unspecified,
            )
        })
        .collect()
}

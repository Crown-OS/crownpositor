//! A surface tree's render elements, every surface on a whole physical pixel.
//!
//! The same walk as smithay's `render_elements_from_surface_tree`, except that
//! no surface is left between pixels. A subsurface's logical offset lands
//! between them on a fractional output, and smithay sizes an element by
//! rounding its two edges separately: a quarter of such subsurfaces came out a
//! pixel wider or narrower than the buffer the client drew for them, and a
//! buffer stretched by a pixel is resampled whole, every glyph in it softened.
//!
//! [`transformed_surface_elements`] also carries a [`UniformAffine`] and an
//! opacity down the tree for `crownos_surface_animation_v1`: an animated
//! subsurface and everything under it are relocated, rescaled about its center
//! and faded. Untransformed trees come out as the plain elements wrapped at
//! scale 1, which draws and damages exactly like the bare element.

use protocols::crownos_surface_animation::surface_transform;
use smithay::{
    backend::renderer::{
        ImportAll, Renderer,
        element::{Kind, surface::WaylandSurfaceRenderElement, utils::RescaleRenderElement},
        utils::RendererSurfaceStateUserData,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Physical, Point, Scale},
    wayland::compositor::{SurfaceData, TraversalAction, with_surface_tree_downward},
};

use crate::utils::affine::UniformAffine;

pub type TransformedSurface<R> = RescaleRenderElement<WaylandSurfaceRenderElement<R>>;

/// Where a surface sits before its animation, and what its ancestors'
/// animations do to it.
#[derive(Debug, Clone, Copy)]
struct Placement {
    location: Point<f64, Physical>,
    affine: UniformAffine,
    alpha: f32,
}

/// The tree's elements as the client committed them, for what no animation
/// reaches: popups, lock surfaces and cursors.
pub fn surface_elements<R>(
    renderer: &mut R,
    root: &WlSurface,
    location: Point<i32, Physical>,
    scale: Scale<f64>,
    alpha: f32,
    kind: Kind,
) -> Vec<WaylandSurfaceRenderElement<R>>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    let mut elements = Vec::new();
    with_surface_tree_downward(
        root,
        location.to_f64(),
        |_, states, parent| match offset_from(states, *parent, scale) {
            Some(location) => TraversalAction::DoChildren(location),
            None => TraversalAction::SkipChildren,
        },
        |surface, states, parent| {
            if let Some(location) = offset_from(states, *parent, scale)
                && let Some(element) =
                    pixel_aligned(renderer, surface, states, location, alpha, kind)
            {
                elements.push(element);
            }
        },
        |_, _, _| true,
    );
    elements
}

/// Where a surface sits, given where its parent does. `None` for a surface
/// with nothing to show, whose children are hidden with it.
fn offset_from(
    states: &SurfaceData,
    parent: Point<f64, Physical>,
    scale: Scale<f64>,
) -> Option<Point<f64, Physical>> {
    let view = states
        .data_map
        .get::<RendererSurfaceStateUserData>()?
        .lock()
        .ok()?
        .view()?;
    Some(parent + view.offset.to_f64().to_physical(scale))
}

/// The surface's element on the whole pixel nearest `location`, where its size
/// is the client's own rounding of the same product.
fn pixel_aligned<R>(
    renderer: &mut R,
    surface: &WlSurface,
    states: &SurfaceData,
    location: Point<f64, Physical>,
    alpha: f32,
    kind: Kind,
) -> Option<WaylandSurfaceRenderElement<R>>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    let snapped = location.to_i32_round::<i32>().to_f64();
    WaylandSurfaceRenderElement::from_surface(renderer, surface, states, snapped, alpha, kind)
        .unwrap_or_else(|err| {
            tracing::warn!(%err, "failed to import a surface");
            None
        })
}

pub fn transformed_surface_elements<R>(
    renderer: &mut R,
    root: &WlSurface,
    location: Point<i32, Physical>,
    scale: Scale<f64>,
    alpha: f32,
    kind: Kind,
) -> Vec<TransformedSurface<R>>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    let mut elements = Vec::new();
    let root_placement = Placement {
        location: location.to_f64(),
        affine: UniformAffine::IDENTITY,
        alpha,
    };

    with_surface_tree_downward(
        root,
        root_placement,
        |_, states, parent| match placement(states, parent, scale) {
            Some(placement) => TraversalAction::DoChildren(placement),
            None => TraversalAction::SkipChildren,
        },
        |surface, states, parent| {
            if let Some(placement) = placement(states, parent, scale)
                && let Some(element) = element(renderer, surface, states, placement, kind)
            {
                elements.push(element);
            }
        },
        |_, _, _| true,
    );
    elements
}

fn placement(states: &SurfaceData, parent: &Placement, scale: Scale<f64>) -> Option<Placement> {
    let view = states
        .data_map
        .get::<RendererSurfaceStateUserData>()?
        .lock()
        .ok()?
        .view()?;
    let location = parent.location + view.offset.to_f64().to_physical(scale);
    let Some(transform) = surface_transform(states) else {
        return Some(Placement {
            location,
            ..*parent
        });
    };
    let size = view.dst.to_f64().to_physical(scale);
    let center = location + Point::new(size.w / 2.0, size.h / 2.0);
    let own = UniformAffine::about(
        center,
        transform.scale,
        transform.translation.to_physical(scale),
    );
    Some(Placement {
        location,
        affine: parent.affine.compose(own),
        alpha: parent.alpha * transform.opacity,
    })
}

/// Built at the affine image of its location divided by the scale, then
/// rescaled about the output origin, which multiplies it back: the element
/// lands on the image with its size scaled.
fn element<R>(
    renderer: &mut R,
    surface: &WlSurface,
    states: &SurfaceData,
    placement: Placement,
    kind: Kind,
) -> Option<TransformedSurface<R>>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    let Placement {
        location,
        affine,
        alpha,
    } = placement;
    if alpha <= 0.0 || affine.scale <= f64::EPSILON {
        return None;
    }
    let relocated = affine.apply(location).downscale(affine.scale);
    pixel_aligned(renderer, surface, states, relocated, alpha, kind)
        .map(|element| RescaleRenderElement::from_element(element, Point::default(), affine.scale))
}

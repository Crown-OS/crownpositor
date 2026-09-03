//! Builds the render element list for one output.
//!
//! Generic over the renderer and over what a backend can do to a window, so
//! adding a backend needs no changes here — it supplies its own renderer and a
//! [`TileDecorator`], and gets the same scene graph.
//!
//! Windows are collected from the shell model rather than through
//! `desktop::space::render_output`: that helper draws its custom elements in
//! front of the space, which is right for Overlay and Top and wrong for Bottom
//! and Background, so a wallpaper would cover the desktop.

pub mod blur;
pub mod cursor;
pub mod decorate;
pub mod decoration;
pub mod element;
pub mod rounded;

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Wrap, surface::WaylandSurfaceRenderElement, utils::CropRenderElement,
        },
    },
    desktop::layer_map_for_output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use crate::{
    rendering::{
        cursor::Cursor,
        decorate::{Backdrop, TileDecorator},
        element::CrownElement,
    },
    shell::{Shell, monitor::Monitor, tile::Tile},
};

/// One output's scene graph, for a given renderer and decorator.
pub type Elements<R, D> = Vec<CrownElement<R, <D as TileDecorator<R>>::Element>>;

/// Front-to-back, the order `OutputDamageTracker::render_output` wants.
pub fn output_elements<R, D>(
    shell: &Shell,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    cursor: &mut Cursor,
    pointer: Point<f64, Logical>,
    scale: Scale<f64>,
    radius: f32,
) -> Elements<R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let mut elements = Elements::<R, D>::new();

    // First in the list is nearest the eye: the cursor is over everything,
    // fullscreen windows included.
    cursor.render(
        &mut elements,
        renderer,
        monitor.geometry().loc,
        pointer,
        scale,
    );

    layer_elements(
        &mut elements,
        monitor,
        renderer,
        decorator,
        scale,
        &[Layer::Overlay, Layer::Top],
    );

    // One workspace once the viewport has settled, two while it is sliding —
    // and the slide is nothing but the offset each tile is drawn at, so the GPU
    // recomposites textures it already holds instead of anyone touching pixels.
    // Layer surfaces sit outside the loop: a bar does not travel with the
    // workspace under it.
    let mut covered = false;
    for (workspace, offset) in monitor.visible_workspaces() {
        match workspace.fullscreen().and_then(|id| workspace.tile(id)) {
            // A fullscreen window covers its page edge to edge, so rounding it
            // would just cut four notches out of the display — and while it is
            // the only page on screen there is nothing behind it to draw.
            Some(tile) => {
                covered |= offset.x == 0.0;
                tile_elements(&mut elements, tile, renderer, decorator, scale, offset, 0.0);
            }
            None => {
                for tile in workspace.stacking_order() {
                    tile_elements(
                        &mut elements,
                        tile,
                        renderer,
                        decorator,
                        scale,
                        offset,
                        radius,
                    );
                }
            }
        }
    }

    if !covered {
        layer_elements(
            &mut elements,
            monitor,
            renderer,
            decorator,
            scale,
            &[Layer::Bottom, Layer::Background],
        );
    }

    let _ = shell;
    elements
}

fn tile_elements<R, D>(
    elements: &mut Elements<R, D>,
    tile: &Tile,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    offset: Point<f64, Logical>,
    radius: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    // The interpolated rect, not the target: this is what makes a window slide.
    // Output-local, because the damage tracker works in this output's own space.
    // `offset` slides the whole workspace the tile belongs to; rounding only at
    // the physical step keeps both motions sub-pixel smooth.
    let rect = tile.render_rect();
    let location: Point<i32, Physical> = (rect.loc + offset).to_physical_precise_round(scale);
    let clip = Rectangle::new(location, rect.size.to_physical_precise_round(scale));
    let size = (clip.size.w as f32, clip.size.h as f32);

    // `Window::render_elements` walks the surface tree and its popups, so popups
    // need no separate pass.
    let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
        tile.window()
            .render_elements(renderer, location, scale, tile.render_alpha());

    for surface in surfaces {
        // A client's buffer is whatever size it last committed — during a shrink
        // still the *old* size — so without the clip it bleeds over its
        // neighbour. `from_element` returns `None` when the element falls
        // entirely outside, which is exactly what should not be drawn.
        let Some(cropped) = CropRenderElement::from_element(surface, scale, clip) else {
            continue;
        };
        if let Some(decorated) = decorator.decorate(renderer, cropped, size, radius) {
            elements.push(CrownElement::Tile(Wrap::from(decorated)));
        }
    }

    // The blurred glass goes in *after* the window's surfaces — later in the
    // list is further from the eye, so it sits directly behind them. `location`
    // is where the surface's own origin lands, which is the space the client
    // expressed its blur region in; `clip` is both the mask the corners are cut
    // from and the bound the region is clipped to.
    if let Some(surface) = blur::window_surface(tile.window()) {
        backdrop_elements(
            elements,
            renderer,
            decorator,
            &surface,
            location,
            scale,
            clip,
            radius,
            tile.render_alpha(),
        );
    }
}

/// Pushes the blurred glass a surface asked for through
/// `ext-background-effect-v1`: one element per rectangle of its committed
/// region, all masked by the same rounded rectangle.
///
/// `origin` is where the surface's own `(0, 0)` lands, and `mask` is the
/// rectangle the region is clipped to and the corners are cut from — a
/// window's animated rect, or a layer surface's geometry.
#[allow(clippy::too_many_arguments)]
fn backdrop_elements<R, D>(
    elements: &mut Elements<R, D>,
    renderer: &mut R,
    decorator: &mut D,
    surface: &WlSurface,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    mask: Rectangle<i32, Physical>,
    radius: f32,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    // Nothing to sample from means nothing to work out: this is the path every
    // frame takes on a backend without a blur pipeline, or with blur off.
    let Some(serial) = decorator.backdrop_source() else {
        return;
    };

    let mut rects = Vec::new();
    let Some(generation) = blur::place_blur_region(surface, origin, scale, mask, &mut rects) else {
        return;
    };

    let (ids, commit) = blur::backdrop_slots(surface, rects.len(), serial, generation);
    for (id, geometry) in std::iter::zip(ids, rects) {
        if let Some(backdrop) = decorator.backdrop(
            renderer,
            Backdrop {
                id,
                commit,
                geometry,
                mask,
                radius,
                alpha,
            },
        ) {
            elements.push(CrownElement::Tile(Wrap::from(backdrop)));
        }
    }
}

fn layer_elements<R, D>(
    elements: &mut Elements<R, D>,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    layers: &[Layer],
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    // A second guard for the same output deadlocks, so keep the scope tight.
    let map = layer_map_for_output(monitor.output());

    for layer in layers {
        for surface in map.layers_on(*layer).rev() {
            let Some(geometry) = map.layer_geometry(surface) else {
                continue;
            };
            let location: Point<i32, Physical> = geometry.loc.to_physical_precise_round(scale);
            let layers: Vec<WaylandSurfaceRenderElement<R>> =
                surface.render_elements(renderer, location, scale, 1.0);
            elements.extend(layers.into_iter().map(CrownElement::Surface));

            // Panels and notifications are what actually wants glass, so layer
            // surfaces get the same treatment windows do — but only above the
            // scene the blur is computed from, or a surface would sample a
            // texture it is itself inside. Square corners: nothing rounds a
            // layer surface here, and the backdrop has to match what is drawn
            // over it.
            if blur::BLURRABLE_LAYERS.contains(layer) {
                let clip = Rectangle::new(location, geometry.size.to_physical_precise_round(scale));
                backdrop_elements(
                    elements,
                    renderer,
                    decorator,
                    surface.wl_surface(),
                    location,
                    scale,
                    clip,
                    0.0,
                    1.0,
                );
            }
        }
    }
}

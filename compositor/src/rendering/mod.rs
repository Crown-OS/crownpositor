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
pub mod element;
pub mod rounded;

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, RenderElement, Wrap, surface::WaylandSurfaceRenderElement,
            utils::CropRenderElement,
        },
    },
    desktop::layer_map_for_output,
    utils::{Logical, Physical, Point, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use crate::{
    rendering::{cursor::Cursor, decorate::TileDecorator, element::CrownElement},
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
    // list is further from the eye, so it sits directly behind them. The
    // committed region currently only gates the effect; the backdrop covers
    // the whole tile, because a window asking to blur anything asks to blur
    // itself in practice, and one rect keeps the corner mask aligned with the
    // window's own rounding.
    if blur::window_blur_bounds(tile.window()).is_some()
        && let Some(id) = blur::backdrop_id(tile.window())
        && let Some(backdrop) = decorator.backdrop(renderer, id, clip, radius, tile.render_alpha())
    {
        elements.push(CrownElement::Tile(Wrap::from(backdrop)));
    }
}

fn layer_elements<R, E>(
    elements: &mut Vec<CrownElement<R, E>>,
    monitor: &Monitor,
    renderer: &mut R,
    scale: Scale<f64>,
    layers: &[Layer],
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    E: RenderElement<R>,
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
        }
    }
}

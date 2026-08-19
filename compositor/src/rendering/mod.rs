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

pub mod decorate;
pub mod element;
pub mod rounded;

use smithay::{
    backend::renderer::{
        element::{
            surface::WaylandSurfaceRenderElement, utils::CropRenderElement, AsRenderElements,
            RenderElement, Wrap,
        },
        ImportAll, Renderer,
    },
    desktop::layer_map_for_output,
    utils::{Physical, Point, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use crate::{
    rendering::{decorate::TileDecorator, element::CrownElement},
    shell::{monitor::Monitor, tile::Tile, Shell},
};

/// One output's scene graph, for a given renderer and decorator.
pub type Elements<R, D> = Vec<CrownElement<R, <D as TileDecorator<R>>::Element>>;

/// Front-to-back, the order `OutputDamageTracker::render_output` wants.
pub fn output_elements<R, D>(
    shell: &Shell,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    radius: f32,
) -> Elements<R, D>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    let mut elements = Elements::<R, D>::new();
    let workspace = monitor.active();

    layer_elements(
        &mut elements,
        monitor,
        renderer,
        scale,
        &[Layer::Overlay, Layer::Top],
    );

    if let Some(tile) = workspace.fullscreen().and_then(|id| workspace.tile(id)) {
        // A fullscreen window covers the screen edge to edge, so rounding it
        // would just cut four notches out of the display.
        tile_elements(&mut elements, tile, renderer, decorator, scale, 0.0);
        return elements;
    }

    for tile in workspace.stacking_order() {
        tile_elements(&mut elements, tile, renderer, decorator, scale, radius);
    }

    layer_elements(
        &mut elements,
        monitor,
        renderer,
        scale,
        &[Layer::Bottom, Layer::Background],
    );

    let _ = shell;
    elements
}

fn tile_elements<R, D>(
    elements: &mut Elements<R, D>,
    tile: &Tile,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    radius: f32,
) where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    // The interpolated rect, not the target: this is what makes a window slide.
    // Output-local, because the damage tracker works in this output's own space.
    let rect = tile.render_rect();
    let location: Point<i32, Physical> = rect.loc.to_physical_precise_round(scale);
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
}

fn layer_elements<R, E>(
    elements: &mut Vec<CrownElement<R, E>>,
    monitor: &Monitor,
    renderer: &mut R,
    scale: Scale<f64>,
    layers: &[Layer],
) where
    R: Renderer + ImportAll,
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

//! Drawing one output's overview.
//!
//! The bridge between the shell model and `spacecontrol::render`: it collects
//! the windows the overview is showing and hands the crate's element builder a
//! closure that applies whatever this backend's renderer can do to them.
//!
//! The closure is the whole seam. `spacecontrol` cannot name [`TileDecorator`]
//! — that type reaches into the blur pipeline and the shader set, which are the
//! compositor's — but it does not need to: it only needs *something* that turns
//! a scaled surface into a drawable one.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Kind, Wrap,
            memory::MemoryRenderBufferRenderElement,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
    },
    desktop::layer_map_for_output,
    utils::{Physical, Point, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use spacecontrol::{
    render::{self, OverviewElement, Preview, Thumb},
    scene,
};

use crate::{
    rendering::{
        Elements, FrameStyle,
        decorate::{Backdrop, TileDecorator},
        element::CrownElement,
        logical,
    },
    shell::monitor::Monitor,
};

/// Appends everything the overview draws on this output.
///
/// Nothing is collected when the overview is fully closed, so the desktop path
/// pays a single boolean for the feature existing.
pub fn overview_elements<R, D>(
    elements: &mut Elements<R, D>,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    radius: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let space = monitor.spacecontrol();
    let geometry = monitor.geometry();

    // The grid is laid out over the active workspace's tiles, in the order the
    // layout was solved from, so index `n` here is index `n` there.
    let grid: Vec<Thumb<'_>> = monitor
        .active()
        .tiles()
        .iter()
        .zip(space.grid())
        .map(|(tile, target)| Thumb {
            window: tile.window(),
            live: tile.render_rect(),
            grid: *target,
            alpha: tile.render_alpha(),
        })
        .collect();

    // Each preview needs its workspace's windows and where they sit in it. The
    // borrow has to outlive the call, so the lists are built first and the
    // previews point into them.
    let windows: Vec<Vec<_>> = monitor
        .workspaces()
        .iter()
        .map(|workspace| {
            workspace
                .stacking_order()
                .rev()
                .map(|tile| (tile.window(), tile.target()))
                .collect()
        })
        .collect();

    let previews: Vec<Preview<'_>> = monitor
        .workspaces()
        .iter()
        .zip(space.bar())
        .zip(&windows)
        .enumerate()
        .map(|(index, ((workspace, slot), windows))| Preview {
            slot: *slot,
            area: workspace.output_area(),
            windows,
            active: index == monitor.active_index(),
        })
        .collect();

    let mut overview: Vec<OverviewElement<R, D::Element>> = Vec::new();
    let mut decorate =
        |renderer: &mut R, element, size, radii| decorator.decorate(renderer, element, size, radii);

    render::elements(
        &mut overview,
        space.chrome(),
        renderer,
        &mut decorate,
        geometry,
        &grid,
        &previews,
        space.carrying(),
        space.hovered(),
        space.overview().progress(),
        space.overview().bar(),
        space.metrics(),
        space.palette(),
        scale,
        radius,
    );

    elements.extend(
        overview
            .into_iter()
            .map(|element| CrownElement::Overview(Wrap::from(element))),
    );
}

/// The wallpaper behind the overview: blurred, and creeping towards the viewer.
///
/// Both follow the overview's own progress, so a swipe that stops halfway
/// leaves the wallpaper half blurred and half zoomed rather than snapping
/// between two states.
///
/// The blur is one full-screen pane of the compositor's own glass laid over the
/// background layers. It samples what has already been drawn beneath it, which
/// at this point in the list is exactly the wallpaper and nothing else.
pub fn background_elements<R, D>(
    elements: &mut Elements<R, D>,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let space = monitor.spacecontrol();
    let geometry = monitor.geometry();
    let output: Rectangle<i32, Physical> = Rectangle::new(
        geometry.loc.to_physical_precise_round(scale),
        geometry.size.to_physical_precise_round(scale),
    );

    let blur = space.overview().blur();
    if blur > 0.0 && decorator.blur_fingerprint().is_some() {
        let glass = decorator.glass(scale.x);
        if let Some(pane) = decorator.backdrop(
            renderer,
            Backdrop {
                id: space.chrome().backdrop(),
                commit: space.backdrop_commit(),
                geometry: output,
                // Square: the wallpaper reaches the edges of the screen, and
                // rounding it would cut four notches out of the display.
                mask: output,
                radius: 0.0,
                glass,
                alpha: blur,
            },
        ) {
            elements.push(CrownElement::Tile(Wrap::from(pane)));
        }
    }

    // Scaled about the middle of the output, so the zoom pulls evenly towards
    // the viewer instead of dragging the wallpaper off one corner.
    let centre = output.loc + Point::from((output.size.w / 2, output.size.h / 2));
    let zoom = space.overview().background_scale();

    // A second guard for the same output deadlocks, so keep the scope tight.
    let map = layer_map_for_output(monitor.output());
    for layer in [Layer::Bottom, Layer::Background] {
        for surface in map.layers_on(layer).rev() {
            let Some(geometry) = map.layer_geometry(surface) else {
                continue;
            };
            let location: Point<i32, Physical> = geometry.loc.to_physical_precise_round(scale);
            let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
                surface.render_elements(renderer, location, scale, 1.0);

            for element in surfaces {
                let scaled = RescaleRenderElement::from_element(element, centre, zoom);
                // Zooming pushes the wallpaper past the edges of the screen;
                // the crop is what stops it being drawn over a neighbouring
                // output.
                let Some(cropped) = CropRenderElement::from_element(scaled, scale, output) else {
                    continue;
                };
                elements.push(CrownElement::Scaled(cropped));
            }
        }
    }
}

/// The workspaces' names, under their previews.
///
/// The last workspace is always the empty one the model keeps spare, so it is
/// drawn as a `+`: the place a window goes to start a new workspace, which is
/// exactly what dropping one on it does.
pub fn label_elements<R, D>(
    elements: &mut Elements<R, D>,
    monitor: &Monitor,
    renderer: &mut R,
    scale: Scale<f64>,
    style: &mut FrameStyle<'_>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let space = monitor.spacecontrol();
    let reveal = space.overview().bar();
    if reveal <= 0.0 {
        return;
    }

    let climb = scene::climb(monitor.geometry(), space.metrics(), reveal);
    let last = monitor.workspaces().len().saturating_sub(1);
    let colour = [1.0, 1.0, 1.0, 1.0];

    for (index, slot) in space.bar().iter().enumerate() {
        let name = if index == last {
            "+".to_owned()
        } else {
            (index + 1).to_string()
        };

        let Some(label) = style.text.label(&name, scale.x, colour, index != last) else {
            continue;
        };
        let size = logical(label.size);

        // Centred in the space the slot reserved for it, which travels with the
        // preview while the bar is still climbing.
        let centre = Point::<i32, Physical>::from((
            ((slot.label.loc.x + (slot.label.size.w - f64::from(size.w)) / 2.0) * scale.x).round()
                as i32,
            ((slot.label.loc.y + climb + (slot.label.size.h - f64::from(size.h)) / 2.0) * scale.y)
                .round() as i32,
        ));

        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            centre.to_f64(),
            &label.buffer,
            Some(reveal as f32),
            Some(Rectangle::from_size(size.to_f64())),
            Some(size),
            Kind::Unspecified,
        ) {
            elements.push(CrownElement::Memory(element));
        }
    }
}

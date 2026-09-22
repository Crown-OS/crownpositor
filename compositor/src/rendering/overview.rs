//! Drawing one output's overview.
//!
//! The bridge between the shell model and `spacecontrol::render`: it collects
//! the windows the overview is showing and hands the crate's element builder
//! a [`Painter`] of closures that apply whatever this backend's renderer can
//! do to them.
//!
//! The painter is the whole seam. `spacecontrol` cannot name [`TileDecorator`]
//! — that type reaches into the blur pipeline and the shader set, which are
//! the compositor's — but it does not need to: it only needs *something* that
//! turns a rectangle into a drawable one. What comes back through it is the
//! same glass, the same shadows and the same rounding the desktop draws, which
//! is what makes a thumbnail look like the window it stands for and a preview
//! look like the workspace it stands for.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Kind, Wrap,
            memory::MemoryRenderBufferRenderElement,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
        utils::CommitCounter,
    },
    desktop::layer_map_for_output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
    wayland::shell::wlr_layer::Layer,
};

use spacecontrol::{
    render::{self, Behind, Carried, OverviewElement, Painter, Pane, Preview, Scaled, Thumb},
    scene,
};

use crate::{
    rendering::{
        Elements, FrameStyle, backdrop_elements,
        blur::{self, ShadowPiece},
        decorate::{Backdrop, Shadow, TileDecorator},
        decoration::window::Border,
        element::CrownElement,
        logical,
    },
    shell::monitor::Monitor,
};

/// The overview's view of this backend's decorator.
///
/// One value rather than three closures: each of the painter's jobs needs the
/// decorator exclusively, and only a single borrow can hand that out.
struct Decorated<'a, D> {
    decorator: &'a mut D,
    scale: Scale<f64>,
    /// Advances while the wallpaper behind the overview is still coming into
    /// focus. A preview's glass is made of that wallpaper, so without it the
    /// damage tracker sees an element that has not moved and leaves the blur
    /// it was first drawn with on screen for the rest of the animation.
    commit: CommitCounter,
}

impl<R, D> Painter<R> for Decorated<'_, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    type Element = D::Element;

    fn decorate(
        &mut self,
        renderer: &mut R,
        element: Scaled<R>,
        size: (f32, f32),
        radius: [f32; 4],
    ) -> Option<Self::Element> {
        self.decorator.decorate(renderer, element, size, radius)
    }

    fn behind(&mut self, renderer: &mut R, behind: Behind<'_>, out: &mut dyn FnMut(Self::Element)) {
        window_backdrop(out, renderer, self.decorator, behind, self.scale);
    }

    fn pane(&mut self, renderer: &mut R, pane: Pane, out: &mut dyn FnMut(Self::Element)) {
        preview_pane(out, renderer, self.decorator, pane, self.scale, self.commit);
    }
}

/// How far a workspace preview is lifted off the wallpaper, in logical pixels:
/// the offset of its shadow and, times [`SHADOW_TAIL`], how far the shadow
/// spreads.
const PREVIEW_SHADOW: f64 = 10.0;

/// Standard deviations of the gaussian a shadow's element has to hold before
/// its tail is below one step of an 8-bit channel.
const SHADOW_TAIL: f32 = 3.0;

/// The shadow a workspace preview casts. Soft and weak: it is there to lift
/// the preview off the wallpaper, not to be seen.
const SHADOW_COLOUR: [f32; 4] = [0.0, 0.0, 0.0, 0.45];

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
    let carrying = space.carrying();
    // The ring follows the viewport rather than the committed index, so a
    // swipe across the overview is seen to change workspace as it crosses
    // rather than only once the fingers come off.
    let nearest = monitor.switch().position().round().max(0.0) as usize;

    // Every workspace with part of itself on screen, at the offset the
    // viewport spring has it at. The overview slides exactly the way the
    // desktop does — a different destination rectangle per window and nothing
    // resolved again — so a swipe across it costs the GPU one more quad and
    // the CPU nothing.
    let mut grid: Vec<Thumb<'_>> = Vec::new();
    let mut carried: Option<Carried<'_>> = None;

    for (index, offset) in monitor.switch().visible(monitor.workspaces().len()) {
        let Some(workspace) = monitor.workspaces().get(index) else {
            continue;
        };
        let offset = Point::from((offset * monitor.page_stride(), 0.0));
        let rects = space.page_grid(index);
        let active = index == monitor.active_index();

        // Stacking order, topmost first: the overview has to stack the way the
        // desktop does or windows swap depth on the way into the grid.
        for tile in workspace.stacking_order() {
            let Some(slot) = workspace.tiles().iter().position(|it| it.id() == tile.id()) else {
                continue;
            };
            let Some(target) = rects.get(slot) else {
                continue;
            };

            if active && carrying.is_some_and(|(carried, _)| carried == slot) {
                carried = carrying.map(|(_, rect)| Carried {
                    window: tile.window(),
                    rect,
                    alpha: tile.render_alpha(),
                });
                continue;
            }

            grid.push(Thumb {
                window: tile.window(),
                live: offset_rect(tile.render_rect(), offset),
                grid: offset_rect(*target, offset),
                alpha: tile.render_alpha(),
                lift: space.window_lift(index, slot),
            });
        }
    }

    // Each preview needs its workspace's windows and where they sit in it. The
    // borrow has to outlive the call, so the lists are built first and the
    // previews point into them.
    let windows: Vec<Vec<_>> = monitor
        .workspaces()
        .iter()
        .map(|workspace| {
            workspace
                .stacking_order()
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
            active: index == nearest,
            lift: space.workspace_lift(index),
        })
        .collect();

    let mut overview: Vec<OverviewElement<R, D::Element>> = Vec::new();
    let mut painter = Decorated {
        decorator,
        scale,
        commit: space.backdrop_commit(),
    };

    render::elements(
        &mut overview,
        space.chrome(),
        renderer,
        &mut painter,
        space.canvas(),
        &grid,
        &previews,
        carried,
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

fn offset_rect(
    rect: Rectangle<f64, Logical>,
    offset: Point<f64, Logical>,
) -> Rectangle<f64, Logical> {
    Rectangle::new(rect.loc + offset, rect.size)
}

/// The blur a window asked for, placed behind its thumbnail.
///
/// A client states its blur region in its own surface coordinates, so a
/// thumbnail is the same placement at a smaller scale — the shrink factor
/// multiplied into the output's. That is the whole difference between a
/// window's glass on the desktop and the same glass in the overview, which is
/// why a window keeps its background when the overview opens instead of
/// flattening onto the wallpaper.
fn window_backdrop<R, D>(
    out: &mut dyn FnMut(D::Element),
    renderer: &mut R,
    decorator: &mut D,
    behind: Behind<'_>,
    scale: Scale<f64>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let Some(surface) = blur::window_surface(behind.window) else {
        return;
    };
    if behind.natural.w <= 0 || behind.natural.h <= 0 {
        return;
    }

    let shrink = Scale::from((
        scale.x * behind.rect.size.w / f64::from(behind.natural.w),
        scale.y * behind.rect.size.h / f64::from(behind.natural.h),
    ));
    let origin: Point<i32, Physical> = behind.rect.loc.to_physical_precise_round(scale);
    let mask = Rectangle::new(origin, behind.rect.size.to_physical_precise_round(scale));

    backdrop_elements(
        out,
        renderer,
        decorator,
        &surface,
        origin,
        shrink,
        mask,
        behind.radius,
        behind.alpha,
    );
}

/// A workspace preview's own backing: the shadow that lifts it off the
/// wallpaper, the rounded sheet of glass it is made of, and the ring around it
/// when it is the workspace being shown.
///
/// Glass rather than a flat fill because a workspace *is* glass — the
/// wallpaper blurred behind whatever is on it — and a white card would be the
/// one thing on screen that does not look like the desktop it is a copy of.
fn preview_pane<R, D>(
    out: &mut dyn FnMut(D::Element),
    renderer: &mut R,
    decorator: &mut D,
    pane: Pane,
    scale: Scale<f64>,
    commit: CommitCounter,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let geometry: Rectangle<i32, Physical> = Rectangle::new(
        pane.rect.loc.to_physical_precise_round(scale),
        pane.rect.size.to_physical_precise_round(scale),
    );
    if geometry.is_empty() || pane.alpha <= 0.0 {
        return;
    }
    let radius = pane.radius * scale.x as f32;

    if let Some(colour) = pane.ring_colour
        && let Some(ring) = decorator.border(
            renderer,
            Border {
                id: pane.ring,
                commit,
                window: geometry,
                thickness: scale.x as f32 * 2.0,
                radius,
                color: colour,
                alpha: pane.alpha,
            },
        )
    {
        out(ring);
    }

    if let Some(glass) = decorator.backdrop(
        renderer,
        Backdrop {
            id: pane.glass,
            commit,
            geometry,
            mask: geometry,
            radius,
            glass: decorator.glass(scale.x),
            alpha: pane.alpha,
            strength: 1.0,
        },
    ) {
        out(glass);
    }

    let sigma = (PREVIEW_SHADOW * scale.y) as f32;
    let shape = Rectangle::new(
        geometry.loc + Point::from((0, (PREVIEW_SHADOW * scale.y / 2.0).round() as i32)),
        geometry.size,
    );
    let spread = (sigma * SHADOW_TAIL).ceil() as i32;
    if let Some(shadow) = decorator.shadow(
        renderer,
        Shadow {
            id: pane.shadow,
            commit,
            piece: ShadowPiece {
                geometry: Rectangle::new(
                    shape.loc - Point::from((spread, spread)),
                    shape.size + Size::from((spread * 2, spread * 2)),
                ),
                shape,
                radius,
                sigma,
                color: SHADOW_COLOUR,
            },
            alpha: pane.alpha,
        },
    ) {
        out(shadow);
    }
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
                // The radius grows with the swipe rather than a finished blur
                // being faded over a sharp wallpaper, which would read as a
                // double image all the way through the gesture.
                strength: blur,
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

    let climb = scene::climb(space.canvas(), space.metrics(), reveal);
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

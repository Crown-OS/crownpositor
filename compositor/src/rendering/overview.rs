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
    render::{
        self, AddTile, Behind, Carried, OverviewElement, Painter, Pane, Preview, Scaled, Thumb,
    },
    scene,
};

use crate::{
    rendering::{
        Elements, FrameStyle, backdrop_elements,
        blur::{self, GlassKind, ShadowPiece},
        decorate::{Backdrop, Shadow, TileDecorator},
        decoration::{label::Label, window::Border},
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
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
    ) -> Option<Self::Element> {
        self.decorator.decorate(renderer, element, shape, radius)
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
        let active = index == monitor.active_index();

        // Stacking order, topmost first: the overview has to stack the way the
        // desktop does or windows swap depth on the way into the grid.
        for tile in workspace.stacking_order() {
            let Some(slot) = workspace.tiles().iter().position(|it| it.id() == tile.id()) else {
                continue;
            };
            let Some(target) = space.grid_rect(index, slot) else {
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
                grid: offset_rect(target, offset),
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

    // Nearest the eye first: a preview being dragged rides above the rest.
    let lifted = space.lifted_preview();
    let mut previews: Vec<Preview<'_>> = windows
        .iter()
        .enumerate()
        .filter_map(|(index, windows)| {
            Some(Preview {
                index,
                slot: space.preview_slot(index)?,
                area: monitor.usable(),
                inset: f64::from(monitor.gaps().outer),
                windows,
                active: index == nearest,
                lift: space.workspace_lift(index),
                close: space.close_visibility(index),
            })
        })
        .collect();
    if let Some(lifted) = lifted
        && let Some(at) = previews.iter().position(|preview| preview.index == lifted)
    {
        let preview = previews.remove(at);
        previews.insert(0, preview);
    }
    let add = space.add_tile().map(|thumb| AddTile {
        thumb,
        lift: space.add_lift(),
    });

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
        add,
        carried,
        space.overview(),
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
    let corner: Point<i32, Physical> = behind.rect.loc.to_physical_precise_round(scale);
    let mask = Rectangle::new(corner, behind.rect.size.to_physical_precise_round(scale));
    // The surface's own origin, the geometry offset before the corner at the
    // thumbnail's size: the space the client's blur region is in.
    let offset = behind.window.geometry().loc.to_f64();
    let origin = corner
        - Point::<f64, Physical>::from((offset.x * shrink.x, offset.y * shrink.y)).to_i32_round();

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
        GlassKind::Window,
        decorator.blur_strength_for(shrink.x.min(shrink.y) / scale.x),
    );
}

/// A workspace preview's own backing: the shadow that lifts it off the
/// wallpaper, the rounded sheet of glass it is made of, and the ring around it
/// when it is the workspace being shown. The shadow goes in front of the glass
/// with the card cut out of it, so the glass never blurs it into its edges.
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
    let radius = pane.radius;

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
                hole: geometry,
                radius,
                sigma,
                color: SHADOW_COLOUR,
            },
            alpha: pane.alpha,
        },
    ) {
        out(shadow);
    }

    if let Some(glass) = decorator.backdrop(
        renderer,
        Backdrop {
            id: pane.glass,
            commit,
            geometry,
            mask: geometry,
            radius,
            glass: decorator.glass(scale.x * pane.shrink, GlassKind::Window),
            alpha: pane.alpha,
            strength: decorator.blur_strength_for(pane.shrink),
        },
    ) {
        out(glass);
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
        let glass = decorator.glass(scale.x, GlassKind::Window);
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

/// The workspaces' names, under their previews wherever those are drawn.
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
    let colour = [1.0, 1.0, 1.0, 1.0];

    for index in 0..space.bar().len() {
        let Some(slot) = space.preview_slot(index) else {
            continue;
        };
        let name = (index + 1).to_string();
        let Some(label) = style.text.label(&name, scale.x, colour, true) else {
            continue;
        };
        let centre = Point::<f64, Logical>::from((
            slot.label.loc.x + slot.label.size.w / 2.0,
            slot.label.loc.y + climb + slot.label.size.h / 2.0,
        ));
        if let Some(element) = centred_label(renderer, label, centre, scale, reveal as f32) {
            elements.push(CrownElement::Memory(element));
        }
    }
}

/// The '×' on each preview showing one, in front of its disc — so pushed
/// before the overview's own elements.
pub fn close_glyph_elements<R, D>(
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

    for index in 0..space.bar().len() {
        let close = space.close_visibility(index);
        if close <= 0.0 {
            continue;
        }
        let Some(slot) = space.preview_slot(index) else {
            continue;
        };
        let thumb = scene::shown(
            slot.thumb,
            climb,
            space.workspace_lift(index),
            space.metrics(),
        );
        let button = scene::close_button(thumb);
        let Some(glyph) = style.text.label("×", scale.x, [1.0, 1.0, 1.0, 1.0], true) else {
            continue;
        };
        let centre = Point::<f64, Logical>::from((
            button.loc.x + button.size.w / 2.0,
            button.loc.y + button.size.h / 2.0,
        ));
        if let Some(element) =
            centred_label(renderer, glyph, centre, scale, (reveal * close) as f32)
        {
            elements.push(CrownElement::Memory(element));
        }
    }
}

/// A rasterised label centred on `centre`, which is logical and output-local.
fn centred_label<R>(
    renderer: &mut R,
    label: &Label,
    centre: Point<f64, Logical>,
    scale: Scale<f64>,
    alpha: f32,
) -> Option<MemoryRenderBufferRenderElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
{
    let size = logical(label.size);
    let origin = Point::<i32, Physical>::from((
        ((centre.x - f64::from(size.w) / 2.0) * scale.x).round() as i32,
        ((centre.y - f64::from(size.h) / 2.0) * scale.y).round() as i32,
    ));
    MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        origin.to_f64(),
        &label.buffer,
        Some(alpha),
        Some(Rectangle::from_size(size.to_f64())),
        Some(size),
        Kind::Unspecified,
    )
    .ok()
}

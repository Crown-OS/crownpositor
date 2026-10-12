//! A window drawn part-way between its application's icon and itself.
//!
//! The window is drawn whole at one uniform zoom, centred on the morph's rect
//! and cut to it with the morph's corner radius: on the way out of a square
//! icon it shows its middle, and its edges come into view as the rect takes on
//! the window's own proportions. The client's buffers are placed and scaled by
//! the GPU as they are, so nothing is re-rendered and a video keeps playing.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Wrap,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
};

use crate::{
    rendering::{
        Elements, FrameStyle, backdrop_elements,
        blur::{self, GlassKind},
        decorate::{Outline, TileDecorator},
        element::CrownElement,
        frame_backing, frame_overlay,
        snapshot::{Snapshot, SnapshotElement},
    },
    shell::{Shell, morph::Morph, tile::Tile},
};

/// Below this zoom the titlebar's marks — title and controls, which do not
/// scale — fade out, so they never crowd a frame shrunk around them.
const MARKS_FADE_FROM: f64 = 0.7;

/// What fills the window's content.
pub(super) enum Content<'a> {
    /// The client's own surfaces.
    Live,
    /// The last frame of a client that is gone.
    Snapshot(&'a Snapshot),
}

/// The window this frame, laid out at the morph's zoom: output-local.
struct Placement {
    /// What is shown: the morph's rect, which everything is cut to.
    frame: Rectangle<i32, Physical>,
    /// Where the client's content lands, at the zoom.
    content: Rectangle<f64, Physical>,
    /// How much of the titlebar is inside `frame`.
    inset: i32,
    zoom: f64,
    radius: f32,
}

impl Placement {
    fn new(
        tile: &Tile,
        morph: &Morph,
        offset: Point<f64, Logical>,
        scale: Scale<f64>,
        radius: f32,
    ) -> Self {
        let animated = tile.render_rect();
        let window = Rectangle::new(animated.loc + offset, animated.size);
        let shown = morph.rect(window);
        let zoom = (shown.size.w / window.size.w).max(shown.size.h / window.size.h);

        let zoomed = window.size.upscale(zoom);
        let zoomed_loc = shown.loc + (shown.size.to_point() - zoomed.to_point()).downscale(2.0);
        let titlebar = f64::from(tile.insets().top) * zoom;
        let content = Rectangle::new(
            zoomed_loc + Point::from((0.0, titlebar)),
            Size::from((zoomed.w, (zoomed.h - titlebar).max(1.0))),
        );

        let frame = Rectangle::new(
            shown.loc.to_physical_precise_round(scale),
            shown.size.to_physical_precise_round(scale),
        );
        let inset =
            (((content.loc.y - shown.loc.y) * scale.y).round() as i32).clamp(0, frame.size.h);
        Self {
            frame,
            content: content.to_physical(scale),
            inset,
            zoom,
            radius: (morph.radius(f64::from(radius) / scale.x) * scale.x) as f32,
        }
    }

    /// Where the client draws: the frame below the titlebar.
    fn clip(&self) -> Rectangle<i32, Physical> {
        Rectangle::new(
            self.frame.loc + Point::from((0, self.inset)),
            Size::from((self.frame.size.w, (self.frame.size.h - self.inset).max(1))),
        )
    }

    /// Square on top where the titlebar covers the corners.
    fn radii(&self) -> [f32; 4] {
        let top = if self.inset > 0 { 0.0 } else { self.radius };
        [self.radius, top, self.radius, top]
    }
}

/// Draws `tile` part-way through `morph`, front to back: titlebar marks,
/// content, its glass, and the frame's backing.
#[allow(clippy::too_many_arguments)]
pub(super) fn morph_elements<R, D>(
    elements: &mut Elements<R, D>,
    shell: &Shell,
    tile: &Tile,
    morph: &Morph,
    content: Content<'_>,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    offset: Point<f64, Logical>,
    radius: f32,
    style: &mut FrameStyle<'_>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let placement = Placement::new(tile, morph, offset, scale, radius);
    let alpha = tile.render_alpha() * morph.alpha();
    let (frame, inset, corner) = (placement.frame, placement.inset, placement.radius);

    if inset > 0 {
        let marks = ((placement.zoom - MARKS_FADE_FROM) / (1.0 - MARKS_FADE_FROM)).clamp(0.0, 1.0);
        frame_overlay(
            elements,
            shell,
            tile,
            renderer,
            decorator,
            frame,
            inset,
            corner,
            scale,
            style,
            alpha * marks as f32,
        );
    }

    match content {
        Content::Live => live_elements(
            elements, tile, &placement, renderer, decorator, scale, alpha,
        ),
        Content::Snapshot(snapshot) => {
            let clip = placement.clip();
            if let Some(element) = SnapshotElement::new(snapshot, placement.content, clip, alpha)
                && let Some(decorated) =
                    decorator.snapshot(renderer, element, clip, placement.radii())
            {
                elements.push(CrownElement::Tile(Wrap::from(decorated)));
            }
        }
    }

    if inset > 0 {
        frame_backing(
            elements, tile, renderer, decorator, frame, inset, corner, style, alpha,
        );
    }
}

/// The client's surfaces at the zoom, and the glass it asked for behind them.
fn live_elements<R, D>(
    elements: &mut Elements<R, D>,
    tile: &Tile,
    placement: &Placement,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let window = tile.window();
    let geometry = window.geometry();
    let clip = placement.clip();
    let radii = placement.radii();
    let zoom = placement.zoom;
    let content_origin = placement.content.loc.to_i32_round();
    // The geometry's corner lands on the content's; the client's own origin
    // sits the geometry's offset before it, and the zoom scales that too.
    let surface_origin = content_origin - geometry.loc.to_physical_precise_round(scale);

    let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
        window.render_elements(renderer, surface_origin, scale, alpha);
    for surface in surfaces {
        let zoomed = RescaleRenderElement::from_element(surface.into(), content_origin, zoom);
        let Some(cropped) = CropRenderElement::from_element(zoomed, scale, clip) else {
            continue;
        };
        if let Some(decorated) = decorator.decorate(renderer, cropped, clip, radii) {
            elements.push(CrownElement::Tile(Wrap::from(decorated)));
        }
    }

    let Some(surface) = blur::window_surface(window) else {
        return;
    };
    let shrink = Scale::from((scale.x * zoom, scale.y * zoom));
    let offset = geometry.loc.to_f64();
    let origin = content_origin
        - Point::<f64, Physical>::from((offset.x * shrink.x, offset.y * shrink.y)).to_i32_round();
    let strength = decorator.blur_strength_for(zoom);
    backdrop_elements(
        &mut |element| elements.push(CrownElement::Tile(Wrap::from(element))),
        renderer,
        decorator,
        &surface,
        origin,
        shrink,
        clip,
        Outline {
            rect: clip,
            radius: placement.radius,
        },
        alpha,
        GlassKind::Window,
        strength,
    );
}

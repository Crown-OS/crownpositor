//! Turning the overview into render elements.
//!
//! Nothing here draws pixels. A window's thumbnail is the window's own surface
//! with a different destination rectangle, so the GPU recomposites textures it
//! already holds and no part of the overview costs a readback or a copy. That
//! is why the grid can stay live — a video keeps playing in its thumbnail for
//! the same price as playing on the desktop.
//!
//! The compositor supplies the rounding through a `decorate` callback rather
//! than a trait, because the effects it can apply depend on its renderer and
//! that knowledge belongs on its side of the seam.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Id, Kind, RenderElement, Wrap,
            memory::MemoryRenderBufferRenderElement,
            solid::SolidColorRenderElement,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
        utils::CommitCounter,
    },
    desktop::Window,
    utils::{Logical, Physical, Point, Rectangle, Scale},
};

use crate::{
    interaction::Target,
    scene::{self, Metrics, Slot},
};

/// A window's surfaces, scaled to the size they are drawn at and clipped to
/// their thumbnail — structurally the compositor's own `Cropped`, so the
/// decorator it already has takes these unchanged.
pub type Scaled<R> = CropRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<R>>>;

/// What the compositor does to one scaled window: rounds it, shadows it, or
/// hands it straight back.
pub type Decorate<'a, R, E> =
    &'a mut dyn FnMut(&mut R, Scaled<R>, (f32, f32), [f32; 4]) -> Option<E>;

smithay::backend::renderer::element::render_elements! {
    /// Everything the overview draws.
    pub OverviewElement<R, E> where R: ImportAll + ImportMem;
    /// A window, shrunk into its thumbnail and decorated by the compositor.
    /// Wrapped because a bare `E` could unify with another variant.
    Window = Wrap<E>,
    /// Flat colour: the wash over the wallpaper, a preview's backing, the ring
    /// around the active workspace.
    Fill = SolidColorRenderElement,
    /// A workspace's name, rasterised by the compositor.
    Label = MemoryRenderBufferRenderElement<R>,
}

/// One window on its way into or out of the grid.
pub struct Thumb<'a> {
    pub window: &'a Window,
    /// Where the window is on the desktop right now — its animated rect, not
    /// its settled one, so opening the overview mid-slide picks the window up
    /// from where it actually is.
    pub live: Rectangle<f64, Logical>,
    /// Where it lands once the overview is fully open.
    pub grid: Rectangle<f64, Logical>,
    pub alpha: f32,
}

/// One workspace's preview along the bottom.
pub struct Preview<'a> {
    pub slot: Slot,
    /// The workspace's own area, which its windows' positions are relative to.
    pub area: Rectangle<i32, Logical>,
    pub windows: &'a [(&'a Window, Rectangle<i32, Logical>)],
    pub active: bool,
}

/// Colours the overview draws its own furniture in.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// The wash over the wallpaper. Alpha is supplied separately, from the
    /// overview's progress.
    pub dim: [f32; 3],
    /// A workspace preview's backing, seen where no window covers it.
    pub preview: [f32; 4],
    /// The ring around the active workspace.
    pub active: [f32; 4],
    /// The lift under the pointer, painted behind the thumbnail it belongs to.
    pub hover: [f32; 4],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            dim: [0.0, 0.0, 0.0],
            preview: [1.0, 1.0, 1.0, 0.12],
            active: [0.20, 0.51, 0.98, 1.0],
            hover: [1.0, 1.0, 1.0, 0.18],
        }
    }
}

/// Width of the ring around the active workspace, in logical pixels.
const RING: f64 = 3.0;

/// Identities the damage tracker knows the overview's flat quads by.
///
/// They have to outlive a frame: a fresh [`Id`] every frame reads as a brand
/// new element and repaints the whole screen continuously.
#[derive(Debug)]
pub struct Chrome {
    dim: Id,
    hover: Id,
    backdrop: Id,
    slots: Vec<[Id; 2]>,
}

impl Default for Chrome {
    fn default() -> Self {
        Self::new()
    }
}

impl Chrome {
    pub fn new() -> Self {
        Self {
            dim: Id::new(),
            hover: Id::new(),
            backdrop: Id::new(),
            slots: Vec::new(),
        }
    }

    /// Mints identities for however many workspace previews there are now.
    ///
    /// Called when the layout is recomputed rather than while drawing, so the
    /// render pass needs nothing but a shared borrow — and so a workspace
    /// appearing never renumbers the ones already on screen.
    pub fn ensure(&mut self, slots: usize) {
        if self.slots.len() < slots {
            self.slots.resize_with(slots, || [Id::new(), Id::new()]);
        }
    }

    /// The full-screen pane of blurred wallpaper. Drawn by the compositor,
    /// which owns the blur pipeline, but identified here so every quad the
    /// overview puts on screen has one owner.
    pub fn backdrop(&self) -> Id {
        self.backdrop.clone()
    }

    fn dim(&self) -> Id {
        self.dim.clone()
    }

    fn hover(&self) -> Id {
        self.hover.clone()
    }

    /// `[backing, ring]` for one workspace preview. `None` before
    /// [`Chrome::ensure`] has been told the slot exists.
    fn slot(&self, index: usize) -> Option<[Id; 2]> {
        self.slots.get(index).cloned()
    }
}

fn fill(
    id: Id,
    rect: Rectangle<f64, Logical>,
    colour: [f32; 4],
    alpha: f32,
    scale: Scale<f64>,
) -> Option<SolidColorRenderElement> {
    let geometry: Rectangle<i32, Physical> = Rectangle::new(
        rect.loc.to_physical_precise_round(scale),
        rect.size.to_physical_precise_round(scale),
    );
    if geometry.is_empty() || alpha <= 0.0 {
        return None;
    }

    let colour = [colour[0], colour[1], colour[2], colour[3] * alpha];
    Some(SolidColorRenderElement::new(
        id,
        geometry,
        CommitCounter::default(),
        colour,
        Kind::Unspecified,
    ))
}

/// The four sides of a ring, as rectangles. Four thin quads cost less than a
/// shader and read identically at this size.
fn ring(rect: Rectangle<f64, Logical>, width: f64) -> [Rectangle<f64, Logical>; 4] {
    let (x, y, w, h) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
    [
        Rectangle::new(
            (x - width, y - width).into(),
            (w + width * 2.0, width).into(),
        ),
        Rectangle::new((x - width, y + h).into(), (w + width * 2.0, width).into()),
        Rectangle::new((x - width, y).into(), (width, h).into()),
        Rectangle::new((x + w, y).into(), (width, h).into()),
    ]
}

/// Draws one window at `rect`, whatever size that is.
///
/// The window's own surfaces are positioned at the rectangle's origin and then
/// scaled about it, so a thumbnail and a full-size window differ only by the
/// factor handed to [`RescaleRenderElement`].
#[allow(clippy::too_many_arguments)]
fn window_at<R, E>(
    out: &mut Vec<OverviewElement<R, E>>,
    renderer: &mut R,
    decorate: Decorate<'_, R, E>,
    window: &Window,
    rect: Rectangle<f64, Logical>,
    scale: Scale<f64>,
    radius: f32,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    E: RenderElement<R>,
{
    let natural = window.geometry().size;
    if natural.w <= 0 || natural.h <= 0 || rect.size.w <= 0.0 || rect.size.h <= 0.0 {
        return;
    }

    let origin: Point<i32, Physical> = rect.loc.to_physical_precise_round(scale);
    let clip = Rectangle::new(origin, rect.size.to_physical_precise_round(scale));
    let shrink = Scale::from((
        rect.size.w / f64::from(natural.w),
        rect.size.h / f64::from(natural.h),
    ));
    let size = (clip.size.w as f32, clip.size.h as f32);

    let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
        window.render_elements(renderer, origin, scale, alpha);

    for surface in surfaces {
        let scaled = RescaleRenderElement::from_element(surface, origin, shrink);
        let Some(cropped) = CropRenderElement::from_element(scaled, scale, clip) else {
            continue;
        };
        if let Some(decorated) = decorate(renderer, cropped, size, [radius; 4]) {
            out.push(OverviewElement::Window(Wrap::from(decorated)));
        }
    }
}

/// Everything one output's overview draws, nearest the eye first — the order
/// the damage tracker and the renderer both want.
///
/// `progress` carries the windows between the desktop and the grid; `bar` does
/// the same for the workspace strip, which trails slightly behind.
#[allow(clippy::too_many_arguments)]
pub fn elements<R, E>(
    out: &mut Vec<OverviewElement<R, E>>,
    chrome: &Chrome,
    renderer: &mut R,
    decorate: Decorate<'_, R, E>,
    output: Rectangle<i32, Logical>,
    grid: &[Thumb<'_>],
    previews: &[Preview<'_>],
    carried: Option<(usize, Rectangle<f64, Logical>)>,
    hovered: Option<Target>,
    progress: f64,
    bar: f64,
    metrics: &Metrics,
    palette: &Palette,
    scale: Scale<f64>,
    radius: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    E: RenderElement<R>,
{
    let climb = scene::climb(output, metrics, bar);

    for (index, preview) in previews.iter().enumerate() {
        let Some([backing, outline]) = chrome.slot(index) else {
            continue;
        };
        let thumb = Rectangle::new(
            (preview.slot.thumb.loc.x, preview.slot.thumb.loc.y + climb).into(),
            preview.slot.thumb.size,
        );

        if preview.active {
            out.extend(
                ring(thumb, RING)
                    .into_iter()
                    .zip(std::iter::repeat(outline))
                    .filter_map(|(side, id)| fill(id, side, palette.active, bar as f32, scale))
                    .map(OverviewElement::Fill),
            );
        }

        if hovered == Some(Target::Workspace(index))
            && let Some(lift) = fill(
                chrome.hover(),
                scene::lift(thumb, metrics.hover),
                palette.hover,
                bar as f32,
                scale,
            )
        {
            out.push(OverviewElement::Fill(lift));
        }

        for (window, live) in preview.windows {
            window_at(
                out,
                renderer,
                decorate,
                window,
                scene::inside(*live, preview.area, thumb),
                scale,
                radius * 0.4,
                bar as f32,
            );
        }

        if let Some(backing) = fill(backing, thumb, palette.preview, bar as f32, scale) {
            out.push(OverviewElement::Fill(backing));
        }
    }

    // The carried window rides above the bar it is being dropped onto.
    if let Some((index, rect)) = carried
        && let Some(thumb) = grid.get(index)
    {
        window_at(
            out,
            renderer,
            decorate,
            thumb.window,
            rect,
            scale,
            radius,
            thumb.alpha,
        );
    }

    for (index, thumb) in grid.iter().enumerate() {
        if carried.is_some_and(|(carried, _)| carried == index) {
            continue;
        }

        let target = match hovered {
            Some(Target::Window(hovered)) if hovered == index => {
                scene::lift(thumb.grid, metrics.hover * progress)
            }
            _ => thumb.grid,
        };
        window_at(
            out,
            renderer,
            decorate,
            thumb.window,
            scene::between(thumb.live, target, progress),
            scale,
            radius,
            thumb.alpha,
        );
    }

    // Last, so it is behind every window: the wash that pulls the wallpaper
    // back and makes the thumbnails read as the foreground.
    if let Some(dim) = fill(
        chrome.dim(),
        output.to_f64(),
        [palette.dim[0], palette.dim[1], palette.dim[2], 1.0],
        progress as f32,
        scale,
    ) {
        out.push(OverviewElement::Fill(dim));
    }
}

#[cfg(test)]
mod tests {
    use smithay::backend::renderer::element::Element;

    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn chrome_hands_back_the_same_identity_every_frame() {
        let mut chrome = Chrome::new();
        chrome.ensure(3);
        assert_eq!(chrome.dim(), chrome.dim());
        assert_eq!(chrome.hover(), chrome.hover());
        assert_eq!(chrome.slot(2), chrome.slot(2));
    }

    #[test]
    fn a_slot_nobody_announced_is_not_drawn() {
        let chrome = Chrome::new();
        assert!(chrome.slot(0).is_none());
    }

    #[test]
    fn every_slot_and_quad_has_its_own_identity() {
        let mut chrome = Chrome::new();
        chrome.ensure(2);
        let (dim, hover) = (chrome.dim(), chrome.hover());
        let [backing, outline] = chrome.slot(0).expect("slot 0");
        let other = chrome.slot(1).expect("slot 1");

        let all = [
            dim,
            hover,
            backing.clone(),
            outline.clone(),
            other[0].clone(),
            other[1].clone(),
        ];
        for (index, id) in all.iter().enumerate() {
            for peer in &all[index + 1..] {
                assert_ne!(id, peer, "two quads share an identity");
            }
        }
    }

    #[test]
    fn growing_the_bar_keeps_the_identities_already_handed_out() {
        let mut chrome = Chrome::new();
        chrome.ensure(1);
        let first = chrome.slot(0);
        chrome.ensure(6);
        assert_eq!(
            chrome.slot(0),
            first,
            "adding a workspace repainted the rest"
        );
    }

    #[test]
    fn a_ring_surrounds_its_rectangle_without_covering_it() {
        let inner = rect(100.0, 100.0, 200.0, 150.0);
        let sides = ring(inner, 4.0);

        for side in sides {
            assert!(side.size.w > 0.0 && side.size.h > 0.0, "{side:?}");
            let overlaps = side.loc.x + side.size.w > inner.loc.x + 1e-9
                && inner.loc.x + inner.size.w > side.loc.x + 1e-9
                && side.loc.y + side.size.h > inner.loc.y + 1e-9
                && inner.loc.y + inner.size.h > side.loc.y + 1e-9;
            assert!(!overlaps, "{side:?} covers the preview it frames");
        }

        // And together they reach every edge.
        let left = sides.iter().map(|s| s.loc.x).fold(f64::MAX, f64::min);
        let right = sides
            .iter()
            .map(|s| s.loc.x + s.size.w)
            .fold(f64::MIN, f64::max);
        assert!((left - (inner.loc.x - 4.0)).abs() < 1e-9);
        assert!((right - (inner.loc.x + inner.size.w + 4.0)).abs() < 1e-9);
    }

    #[test]
    fn a_transparent_or_empty_fill_is_not_drawn() {
        let scale = Scale::from(1.0);
        assert!(fill(Id::new(), rect(0.0, 0.0, 10.0, 10.0), [1.0; 4], 0.0, scale).is_none());
        assert!(fill(Id::new(), rect(0.0, 0.0, 0.0, 0.0), [1.0; 4], 1.0, scale).is_none());
    }

    #[test]
    fn a_fill_lands_where_it_was_asked_to() {
        let element = fill(
            Id::new(),
            rect(10.0, 20.0, 100.0, 50.0),
            [1.0, 0.0, 0.0, 1.0],
            1.0,
            Scale::from(2.0),
        )
        .expect("a solid quad");

        let geometry = element.geometry(Scale::from(2.0));
        assert_eq!(geometry.loc.x, 20);
        assert_eq!(geometry.loc.y, 40);
        assert_eq!(geometry.size.w, 200);
        assert_eq!(geometry.size.h, 100);
    }

    #[test]
    fn progress_scales_the_wash_rather_than_switching_it_on() {
        let scale = Scale::from(1.0);
        let half = fill(
            Id::new(),
            rect(0.0, 0.0, 100.0, 100.0),
            [0.0, 0.0, 0.0, 1.0],
            0.5,
            scale,
        )
        .expect("a wash");
        assert!((half.color().a() - 0.5).abs() < 1e-6, "{:?}", half.color());
    }
}

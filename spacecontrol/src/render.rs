//! Turning the overview into render elements.
//!
//! Nothing here draws pixels. A window's thumbnail is the window's own surface
//! with a different destination rectangle, so the GPU recomposites textures it
//! already holds and no part of the overview costs a readback or a copy. That
//! is why the grid can stay live — a video keeps playing in its thumbnail for
//! the same price as playing on the desktop.
//!
//! Everything the compositor can do that this crate cannot name reaches it
//! through [`Painter`]: rounding a window, the blurred glass a window asked to
//! stand on, a workspace preview's own material. The effects available depend
//! on the renderer, and that knowledge belongs on the compositor's side of the
//! seam.
//!
//! Depth is the caller's to decide. Windows are drawn in the order they are
//! handed over, nearest the eye first, so the overview stacks exactly the way
//! the desktop does rather than inventing an order of its own.

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
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
};

use crate::scene::{self, Canvas, Metrics, Slot};

/// A window's surfaces, scaled to the size they are drawn at and clipped to
/// their thumbnail — structurally the compositor's own `Cropped`, so the
/// decorator it already has takes these unchanged.
pub type Scaled<R> = CropRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<R>>>;

/// One window's own background effects, at the size its thumbnail is drawn.
///
/// The overview does not know what a client asked for — a blur region, a tint,
/// a shadow — only where the thumbnail landed and how far it was shrunk to get
/// there. The compositor places the effects from that.
pub struct Behind<'a> {
    pub window: &'a Window,
    /// Where the thumbnail is, output-local.
    pub rect: Rectangle<f64, Logical>,
    /// The window's own size, which `rect` is a shrunken copy of.
    pub natural: Size<i32, Logical>,
    pub radius: f32,
    pub alpha: f32,
}

/// A workspace preview's backing: a rounded sheet of the compositor's own
/// glass, the shadow it casts, and the ring around it when it is the active
/// workspace.
///
/// A preview is a small copy of a workspace, so it is made of what a workspace
/// is made of — the wallpaper showing through blurred, not a flat white card.
pub struct Pane {
    pub glass: Id,
    pub shadow: Id,
    pub ring: Id,
    pub rect: Rectangle<f64, Logical>,
    pub radius: f32,
    pub alpha: f32,
    /// The active workspace's ring colour, straight RGBA. `None` leaves the
    /// preview unringed.
    pub ring_colour: Option<[f32; 4]>,
}

/// Everything the compositor lends the overview.
///
/// The backings hand their elements to a sink rather than returning them, so
/// one call can produce a shadow, a sheet of glass and a ring without
/// allocating a vector to carry them back across the seam.
pub trait Painter<R>
where
    R: Renderer + ImportAll,
{
    /// What a drawn thing becomes. The compositor's decorated tile type.
    type Element: RenderElement<R>;

    /// Rounds, shadows or otherwise finishes one scaled window, or hands it
    /// straight back. `None` drops it.
    fn decorate(
        &mut self,
        renderer: &mut R,
        element: Scaled<R>,
        size: (f32, f32),
        radius: [f32; 4],
    ) -> Option<Self::Element>;

    /// The material a window stands on, behind its thumbnail.
    fn behind(&mut self, renderer: &mut R, behind: Behind<'_>, out: &mut dyn FnMut(Self::Element));

    /// A workspace preview's own backing.
    fn pane(&mut self, renderer: &mut R, pane: Pane, out: &mut dyn FnMut(Self::Element));
}

smithay::backend::renderer::element::render_elements! {
    /// Everything the overview draws.
    pub OverviewElement<R, E> where R: ImportAll + ImportMem;
    /// A window, shrunk into its thumbnail — or a piece of the compositor's
    /// own material behind one. Wrapped because a bare `E` could unify with
    /// another variant.
    Window = Wrap<E>,
    /// Flat colour: the wash that pulls the wallpaper back behind the grid.
    Fill = SolidColorRenderElement,
    /// A workspace's name, rasterised by the compositor.
    Label = MemoryRenderBufferRenderElement<R>,
}

/// A window the pointer has picked up off the grid.
///
/// It is not in `grid` at all — the caller leaves it out — because a carried
/// window belongs to the pointer rather than to any workspace's stack.
pub struct Carried<'a> {
    pub window: &'a Window,
    pub rect: Rectangle<f64, Logical>,
    pub alpha: f32,
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
    /// How far the pointer has lifted it, 0 to 1.
    pub lift: f64,
}

/// One workspace's preview along the bottom.
pub struct Preview<'a> {
    pub slot: Slot,
    /// The workspace's own area, which its windows' positions are relative to.
    pub area: Rectangle<i32, Logical>,
    /// Topmost first, so the preview stacks the way the workspace does.
    pub windows: &'a [(&'a Window, Rectangle<i32, Logical>)],
    pub active: bool,
    /// How far the pointer has lifted it, 0 to 1.
    pub lift: f64,
}

/// Colours the overview draws its own furniture in.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// The wash over the wallpaper. Alpha is supplied separately, from the
    /// overview's progress.
    pub dim: [f32; 3],
    /// The ring around the active workspace.
    pub active: [f32; 4],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            dim: [0.0, 0.0, 0.0],
            active: [0.20, 0.51, 0.98, 1.0],
        }
    }
}

/// Identities the damage tracker knows the overview's own quads by.
///
/// They have to outlive a frame: a fresh [`Id`] every frame reads as a brand
/// new element and repaints the whole screen continuously.
#[derive(Debug, Clone)]
struct SlotIds {
    glass: Id,
    shadow: Id,
    ring: Id,
}

impl SlotIds {
    fn new() -> Self {
        Self {
            glass: Id::new(),
            shadow: Id::new(),
            ring: Id::new(),
        }
    }
}

#[derive(Debug)]
pub struct Chrome {
    dim: Id,
    backdrop: Id,
    slots: Vec<SlotIds>,
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
            self.slots.resize_with(slots, SlotIds::new);
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

    /// `None` before [`Chrome::ensure`] has been told the slot exists.
    fn slot(&self, index: usize) -> Option<&SlotIds> {
        self.slots.get(index)
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

/// Draws one window at `rect`, whatever size that is, on the material it
/// asked to stand on.
///
/// The window's own surfaces are positioned at the rectangle's origin and then
/// scaled about it, so a thumbnail and a full-size window differ only by the
/// factor handed to [`RescaleRenderElement`].
#[allow(clippy::too_many_arguments)]
fn window_at<R, P>(
    out: &mut Vec<OverviewElement<R, P::Element>>,
    renderer: &mut R,
    painter: &mut P,
    window: &Window,
    rect: Rectangle<f64, Logical>,
    scale: Scale<f64>,
    radius: f32,
    alpha: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    P: Painter<R>,
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
        if let Some(decorated) = painter.decorate(renderer, cropped, size, [radius; 4]) {
            out.push(OverviewElement::Window(Wrap::from(decorated)));
        }
    }

    // After the surfaces, so it lands directly behind them — a translucent
    // window keeps the glass it has on the desktop instead of losing it the
    // moment the overview opens.
    painter.behind(
        renderer,
        Behind {
            window,
            rect,
            natural,
            radius,
            alpha,
        },
        &mut |element| out.push(OverviewElement::Window(Wrap::from(element))),
    );
}

/// Everything one output's overview draws, nearest the eye first — the order
/// the damage tracker and the renderer both want.
///
/// `grid` and each preview's windows are drawn in the order they are given, so
/// the caller's stacking order is the one that reaches the screen.
///
/// `progress` carries the windows between the desktop and the grid; `bar` does
/// the same for the workspace strip, which trails slightly behind.
#[allow(clippy::too_many_arguments)]
pub fn elements<R, P>(
    out: &mut Vec<OverviewElement<R, P::Element>>,
    chrome: &Chrome,
    renderer: &mut R,
    painter: &mut P,
    canvas: Canvas,
    grid: &[Thumb<'_>],
    previews: &[Preview<'_>],
    carried: Option<Carried<'_>>,
    progress: f64,
    bar: f64,
    metrics: &Metrics,
    palette: &Palette,
    scale: Scale<f64>,
    radius: f32,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    P: Painter<R>,
{
    let climb = scene::climb(canvas, metrics, bar);

    for (index, preview) in previews.iter().enumerate() {
        let Some(ids) = chrome.slot(index) else {
            continue;
        };
        let thumb = scene::lift(
            Rectangle::new(
                (preview.slot.thumb.loc.x, preview.slot.thumb.loc.y + climb).into(),
                preview.slot.thumb.size,
            ),
            metrics.hover * preview.lift,
        );

        for (window, live) in preview.windows {
            window_at(
                out,
                renderer,
                painter,
                window,
                scene::inside(*live, preview.area, thumb),
                scale,
                radius * PREVIEW_WINDOW_RADIUS,
                bar as f32,
            );
        }

        painter.pane(
            renderer,
            Pane {
                glass: ids.glass.clone(),
                shadow: ids.shadow.clone(),
                ring: ids.ring.clone(),
                rect: thumb,
                radius: radius * PREVIEW_RADIUS,
                alpha: bar as f32,
                ring_colour: preview.active.then_some(palette.active),
            },
            &mut |element| out.push(OverviewElement::Window(Wrap::from(element))),
        );
    }

    // The carried window rides above the bar it is being dropped onto.
    if let Some(carried) = carried {
        window_at(
            out,
            renderer,
            painter,
            carried.window,
            carried.rect,
            scale,
            radius,
            carried.alpha,
        );
    }

    for thumb in grid {
        // The lift is scaled by how far open the overview is, so a window
        // under the pointer on the way in grows with everything else rather
        // than jumping out of a grid that has not arrived.
        let target = scene::lift(thumb.grid, metrics.hover * thumb.lift * progress);
        window_at(
            out,
            renderer,
            painter,
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
        canvas.output.to_f64(),
        [palette.dim[0], palette.dim[1], palette.dim[2], 1.0],
        progress as f32,
        scale,
    ) {
        out.push(OverviewElement::Fill(dim));
    }
}

/// How round a workspace preview's corners are, against a window's own radius.
/// Squarer than a window: the preview stands for the whole screen, and a
/// screen's corners are the display's, not a window's.
const PREVIEW_RADIUS: f32 = 0.6;

/// How much of a window's corner radius survives into a workspace preview.
/// The preview is a small copy of the screen, so its windows round by about as
/// much as they are shrunk.
const PREVIEW_WINDOW_RADIUS: f32 = 0.4;

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
        assert_eq!(chrome.slot(2).map(|ids| ids.glass.clone()), {
            chrome.slot(2).map(|ids| ids.glass.clone())
        });
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
        let first = chrome.slot(0).expect("slot 0");
        let second = chrome.slot(1).expect("slot 1");

        let all = [
            chrome.dim(),
            chrome.backdrop(),
            first.glass.clone(),
            first.shadow.clone(),
            first.ring.clone(),
            second.glass.clone(),
            second.shadow.clone(),
            second.ring.clone(),
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
        let first = chrome.slot(0).expect("slot 0").glass.clone();
        chrome.ensure(6);
        assert_eq!(
            chrome.slot(0).expect("slot 0").glass,
            first,
            "adding a workspace repainted the rest"
        );
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

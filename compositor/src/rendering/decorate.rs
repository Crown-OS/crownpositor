//! The seam between "what to draw" and "what this renderer can do to it".
//!
//! `rendering` must not know about any particular renderer: a backend added
//! later brings its own, and rounding corners with a GLES texture-program
//! override is not something every renderer can do. So a backend supplies a
//! decorator, and gets whatever effects its renderer supports — [`PassThrough`]
//! if none.

use crate::rendering::{
    blur::{Glass, GlassKind, ShadowPiece},
    decoration::{Border, TitleBarParams},
    snapshot::SnapshotElement,
    surface_tree::TransformedSurface,
    warp::WarpRequest,
};

use smithay::{
    backend::renderer::{
        ImportAll, Renderer,
        element::{
            Element, Id, RenderElement, memory::MemoryRenderBufferRenderElement,
            utils::CropRenderElement,
        },
        utils::CommitCounter,
    },
    utils::{Physical, Rectangle, Scale},
};

/// A tile's surfaces, scaled to the size they are drawn at and clipped to the
/// window's animated rect.
///
/// The rescale is what lets the same element — and so the same decorator, the
/// same shaders and the same damage tracking — draw a window both at its own
/// size on the desktop and shrunk into an overview thumbnail. On the desktop
/// the factor is 1.0, which the wrapper resolves to a no-op.
pub type Cropped<R> = CropRenderElement<TransformedSurface<R>>;

/// One piece of blurred glass to draw behind a surface.
///
/// A piece rather than a whole window because both blur protocols let a client
/// blur *part* of itself: a `wl_region` with a hole in it, or a shape built out
/// of several primitives, produces several of these.
#[derive(Debug, Clone)]
pub struct Backdrop {
    /// Stable across frames for the same piece of the same surface, or the
    /// damage tracker treats every frame's backdrop as a brand new element and
    /// repaints the window's area continuously.
    pub id: Id,
    /// Changes when the blur settings or the client's region do — everything
    /// else about this backdrop's pixels the damage tracker already sees.
    pub commit: CommitCounter,
    /// The rectangle to fill, in output-local physical coordinates.
    pub geometry: Rectangle<i32, Physical>,
    /// The rectangle the corner rounding is measured against: the whole
    /// window, so that pieces of one region round as one shape instead of each
    /// growing its own four corners.
    pub mask: Rectangle<i32, Physical>,
    pub radius: f32,
    /// What the glass is made of: its tint, its vibrancy and the width of the
    /// refractive rim along its edge.
    pub glass: Glass,
    pub alpha: f32,
    /// How much of the configured blur to apply, 0 to 1. Anything below the
    /// full strength is a blur being swiped in: the pyramid is run shallower
    /// and narrower rather than in full, so the radius grows with the gesture
    /// and a weak blur costs weak-blur work.
    pub strength: f32,
}

/// The rounded rectangle a piece of glass is cut from and lit as, in
/// output-local physical coordinates. Larger than the glass itself wherever the
/// glass is one part of a bigger shape, like a client's blur inside its frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outline {
    pub rect: Rectangle<i32, Physical>,
    pub radius: f32,
}

/// One blurred silhouette to draw underneath a surface, and the identity the
/// damage tracker knows it by.
#[derive(Debug, Clone)]
pub struct Shadow {
    pub id: Id,
    pub commit: CommitCounter,
    pub piece: ShadowPiece,
    pub alpha: f32,
}

/// How to draw one lock-screen clock glyph: bent, blurred, and maybe glass.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphDraw {
    /// Column-major: the element's own UV to the sprite's UV, which undoes
    /// the clock's warp.
    pub unwarp: [f32; 9],
    /// Blur radius in sprite UV units, per axis.
    pub reach: [f32; 2],
    /// Frosted glass showing the background through the glyph, whose sprite
    /// is then its coverage in white.
    pub glass: Option<GlyphGlass>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlyphGlass {
    /// Stable for as long as the glyph's sprite lives.
    pub id: Id,
    /// Where the glyph's element draws, output-local.
    pub geometry: Rectangle<i32, Physical>,
    /// The lock's dim, when the glass behind the clock already carries it.
    pub dim: f32,
    /// How white the glass washes at the glyph's top, and at its baseline.
    pub wash: [f32; 2],
    /// The sprite rows the glyph's top and baseline sit on.
    pub rows: [f32; 2],
    /// How far the lit top edge reaches, in sprite rows.
    pub edge: f32,
}

impl GlyphDraw {
    /// Changes whenever the draw does.
    pub fn fingerprint(&self) -> u64 {
        let glass = self.glass.iter().flat_map(|glass| {
            [glass.dim, glass.edge]
                .into_iter()
                .chain(glass.wash)
                .chain(glass.rows)
        });
        self.unwarp
            .iter()
            .chain(&self.reach)
            .copied()
            .chain(glass)
            .fold(0xcbf2_9ce4_8422_2325, |hash, value| {
                (hash ^ u64::from(value.to_bits())).wrapping_mul(0x0100_0000_01b3)
            })
    }
}

/// Applies a backend's per-window effects to a tile.
pub trait TileDecorator<R>
where
    R: Renderer + ImportAll,
{
    /// What the decorated element becomes. `Cropped<R>` for a decorator that
    /// adds nothing.
    type Element: RenderElement<R>;

    /// `None` drops the element. Returning the input undecorated is the right
    /// answer when an effect is unavailable — square corners beat no window.
    /// `shape` is the window the element belongs to, output-local: the rect
    /// its corners are cut from, whatever part of it this one surface covers.
    fn decorate(
        &mut self,
        renderer: &mut R,
        element: Cropped<R>,
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
    ) -> Option<Self::Element>;

    /// A closed window's last frame, rounded like the live window was. `None`
    /// — the default — lets it vanish, which is what a renderer that cannot
    /// keep a texture of its own offers.
    fn snapshot(
        &mut self,
        renderer: &mut R,
        element: SnapshotElement,
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
    ) -> Option<Self::Element> {
        let _ = (renderer, element, shape, radius);
        None
    }

    /// A window bent on the GPU — poured into its icon or crumbled to dust.
    /// `None` — the default — makes the caller draw the flat morph, or let a
    /// crumbling window go.
    fn warp(&mut self, renderer: &mut R, request: WarpRequest<'_, '_>) -> Option<Self::Element> {
        let _ = (renderer, request);
        None
    }

    /// One rectangle of the blurred glass drawn *behind* a surface that
    /// committed a blur region. `None` — the default, and the only answer a
    /// decorator without a blur pipeline has — simply leaves the surface
    /// without the effect.
    fn backdrop(&mut self, renderer: &mut R, backdrop: Backdrop) -> Option<Self::Element> {
        let _ = (renderer, backdrop);
        None
    }

    /// One blurred silhouette cast under a surface that asked for a shadow.
    /// `None` — the default — leaves the surface without one.
    fn shadow(&mut self, renderer: &mut R, shadow: Shadow) -> Option<Self::Element> {
        let _ = (renderer, shadow);
        None
    }

    /// A floating window's titlebar: its tint, its highlight and its three
    /// controls, generated from geometry in one pass. `None` — the default —
    /// leaves the window undecorated, which is what a renderer with no custom
    /// shaders can offer.
    fn title_bar(&mut self, renderer: &mut R, params: TitleBarParams) -> Option<Self::Element> {
        let _ = (renderer, params);
        None
    }

    /// The hairline ring around a floating window. `None` — the default —
    /// leaves it without one.
    fn border(&mut self, renderer: &mut R, border: Border) -> Option<Self::Element> {
        let _ = (renderer, border);
        None
    }

    /// A compositor-drawn sprite blurred by `radius` texels. `None` — the
    /// default — lets the caller draw it sharp.
    fn blurred_sprite(
        &mut self,
        renderer: &mut R,
        sprite: MemoryRenderBufferRenderElement<R>,
        radius: f32,
    ) -> Option<Self::Element> {
        let _ = (renderer, sprite, radius);
        None
    }

    /// A lock-screen clock glyph drawn as `glyph` says. `None` — the default —
    /// lets the caller draw it flat.
    fn lock_glyph(
        &mut self,
        renderer: &mut R,
        sprite: MemoryRenderBufferRenderElement<R>,
        glyph: GlyphDraw,
    ) -> Option<Self::Element> {
        let _ = (renderer, sprite, glyph);
        None
    }

    /// The material this decorator makes the compositor's own glass out of —
    /// window frames, menus, snap previews — at one output's scale. A surface
    /// that asked for its own through `crownos_background_effects` gets that
    /// instead, and never goes through here.
    fn glass(&self, scale: f64, kind: GlassKind) -> Glass {
        let _ = (scale, kind);
        Glass::default()
    }

    /// How strongly to blur glass shown at `shrink` of its size, so a
    /// thumbnail's glass is the desktop's blurred that much less.
    fn blur_strength_for(&self, shrink: f64) -> f32 {
        let _ = shrink;
        1.0
    }

    /// Identifies the settings this decorator would draw backdrops with,
    /// changing whenever one of them does.
    ///
    /// `None` — the default — means it draws none this frame, and the caller
    /// can skip working out where backdrops would go at all.
    fn blur_fingerprint(&self) -> Option<u64> {
        None
    }
}

/// Surfaces nothing clips or rounds — popups, square panels — decorated anyway,
/// at zero radius and cropped to nothing but themselves: whatever the
/// decorator does to the pixels it samples, such as resampling a buffer drawn
/// at another size, they get as windows do.
pub fn unclipped<R, D>(
    renderer: &mut R,
    decorator: &mut D,
    surfaces: impl IntoIterator<Item = TransformedSurface<R>>,
    scale: Scale<f64>,
) -> Vec<D::Element>
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
    D: TileDecorator<R>,
{
    surfaces
        .into_iter()
        .filter_map(|surface| {
            let geometry = surface.geometry(scale);
            let cropped = CropRenderElement::from_element(surface, scale, geometry)?;
            decorator.decorate(renderer, cropped, geometry, [0.0; 4])
        })
        .collect()
}

/// Draws windows as they are. What a renderer without custom shaders uses.
#[derive(Debug, Default, Clone, Copy)]
pub struct PassThrough;

impl<R> TileDecorator<R> for PassThrough
where
    R: Renderer + ImportAll,
    R::TextureId: Clone + 'static,
{
    type Element = Cropped<R>;

    fn decorate(
        &mut self,
        _renderer: &mut R,
        element: Cropped<R>,
        _shape: Rectangle<i32, Physical>,
        _radius: [f32; 4],
    ) -> Option<Self::Element> {
        Some(element)
    }
}

#[cfg(test)]
mod tests {
    use smithay::backend::renderer::gles::GlesRenderer;

    use super::*;
    use crate::rendering::{element::CrownElement, rounded::GlesDecorator};

    /// Compile-time proof that the seam takes more than one decorator.
    ///
    /// If a future backend's decorator does not fit, this stops compiling — which
    /// is the point: the claim is that `rendering` needs no changes to gain one.
    fn assert_usable<R, D>()
    where
        R: Renderer + ImportAll,
        R::TextureId: Clone + 'static,
        D: TileDecorator<R>,
    {
    }

    #[test]
    fn both_decorators_satisfy_the_seam() {
        assert_usable::<GlesRenderer, PassThrough>();
        assert_usable::<GlesRenderer, GlesDecorator<'static>>();
    }

    #[test]
    fn a_scene_graph_exists_for_each() {
        // The element enum has to accept both decorated tile types.
        fn accepts<R, D>(_: Vec<CrownElement<R, D::Element>>)
        where
            R: Renderer + ImportAll,
            D: TileDecorator<R>,
        {
        }
        accepts::<GlesRenderer, PassThrough>(Vec::new());
        accepts::<GlesRenderer, GlesDecorator<'static>>(Vec::new());
    }
}

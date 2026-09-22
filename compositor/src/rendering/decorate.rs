//! The seam between "what to draw" and "what this renderer can do to it".
//!
//! `rendering` must not know about any particular renderer: a backend added
//! later brings its own, and rounding corners with a GLES texture-program
//! override is not something every renderer can do. So a backend supplies a
//! decorator, and gets whatever effects its renderer supports — [`PassThrough`]
//! if none.

use crate::rendering::{
    blur::{Glass, ShadowPiece},
    decoration::{Border, TitleBarParams},
};

use smithay::{
    backend::renderer::{
        ImportAll, Renderer,
        element::{
            Id, RenderElement, surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
        utils::CommitCounter,
    },
    utils::{Physical, Rectangle},
};

/// A tile's surfaces, scaled to the size they are drawn at and clipped to the
/// window's animated rect.
///
/// The rescale is what lets the same element — and so the same decorator, the
/// same shaders and the same damage tracking — draw a window both at its own
/// size on the desktop and shrunk into an overview thumbnail. On the desktop
/// the factor is 1.0, which the wrapper resolves to a no-op.
pub type Cropped<R> = CropRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<R>>>;

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
    fn decorate(
        &mut self,
        renderer: &mut R,
        element: Cropped<R>,
        size: (f32, f32),
        radius: [f32; 4],
    ) -> Option<Self::Element>;

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

    /// The material this decorator makes the compositor's own glass out of —
    /// window frames, menus, snap previews — at one output's scale. A surface
    /// that asked for its own through `crownos_background_effects` gets that
    /// instead, and never goes through here.
    fn glass(&self, scale: f64) -> Glass {
        let _ = scale;
        Glass::default()
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
        _size: (f32, f32),
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

//! The seam between "what to draw" and "what this renderer can do to it".
//!
//! `rendering` must not know about any particular renderer: a backend added
//! later brings its own, and rounding corners with a GLES texture-program
//! override is not something every renderer can do. So a backend supplies a
//! decorator, and gets whatever effects its renderer supports — [`PassThrough`]
//! if none.

use smithay::{
    backend::renderer::{
        ImportAll, Renderer,
        element::{
            Id, RenderElement, surface::WaylandSurfaceRenderElement, utils::CropRenderElement,
        },
    },
    utils::{Physical, Rectangle},
};

/// A tile's surfaces, already clipped to the window's animated rect.
pub type Cropped<R> = CropRenderElement<WaylandSurfaceRenderElement<R>>;

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
        radius: f32,
    ) -> Option<Self::Element>;

    /// The blurred glass drawn *behind* a tile whose surface committed a blur
    /// region. `None` — the default, and the only answer a decorator without
    /// a blur pipeline has — simply leaves the window without the effect.
    ///
    /// `id` must be stable per window across frames, or the damage tracker
    /// repaints the backdrop every frame.
    fn backdrop(
        &mut self,
        renderer: &mut R,
        id: Id,
        geometry: Rectangle<i32, Physical>,
        radius: f32,
        alpha: f32,
    ) -> Option<Self::Element> {
        let _ = (renderer, id, geometry, radius, alpha);
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
        _radius: f32,
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
        assert_usable::<GlesRenderer, GlesDecorator>();
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
        accepts::<GlesRenderer, GlesDecorator>(Vec::new());
    }
}

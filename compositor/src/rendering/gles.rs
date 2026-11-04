//! The renderers the compositor's shader elements can draw through.
//!
//! Every effect here is a GLES program: a texture-program override, a pixel
//! program or raw GL against the frame's context. Smithay's generic `Frame`
//! offers none of that, so an element cannot be written against an arbitrary
//! renderer — but it need not be written twice either. The two renderers in
//! use both sit on a [`GlesRenderer`]: the plain one on winit, and the
//! multi-GPU one on KMS, which composites on one GPU's GLES context and
//! exposes it through `AsMut`. This trait is that one seam, and every element
//! and the decorator are generic over it.

use smithay::backend::renderer::{
    ImportAll, ImportMem, Renderer,
    gles::{GlesError, GlesFrame, GlesRenderer},
    multigpu::Error as MultiError,
};

use crate::backend::render::KmsRenderer;

/// A renderer whose frames are, underneath, a [`GlesFrame`].
pub trait GlesBacked:
    Renderer<TextureId: Send + Clone + 'static> + ImportAll + ImportMem + Sized
{
    fn gles(&mut self) -> &mut GlesRenderer;

    fn gles_frame<'a, 'frame, 'buffer>(
        frame: &'a mut Self::Frame<'frame, 'buffer>,
    ) -> &'a mut GlesFrame<'frame, 'buffer>
    where
        'buffer: 'frame,
        Self: 'frame;

    fn lift(err: GlesError) -> Self::Error;
}

impl GlesBacked for GlesRenderer {
    fn gles(&mut self) -> &mut GlesRenderer {
        self
    }

    fn gles_frame<'a, 'frame, 'buffer>(
        frame: &'a mut Self::Frame<'frame, 'buffer>,
    ) -> &'a mut GlesFrame<'frame, 'buffer>
    where
        'buffer: 'frame,
        Self: 'frame,
    {
        frame
    }

    fn lift(err: GlesError) -> Self::Error {
        err
    }
}

impl<'render> GlesBacked for KmsRenderer<'render> {
    fn gles(&mut self) -> &mut GlesRenderer {
        self.as_mut()
    }

    /// The render device's GLES frame: the context the shaders were compiled
    /// in and the one every pyramid, canvas and snapshot lives on, so nothing
    /// drawn through it ever crosses GPUs.
    fn gles_frame<'a, 'frame, 'buffer>(
        frame: &'a mut Self::Frame<'frame, 'buffer>,
    ) -> &'a mut GlesFrame<'frame, 'buffer>
    where
        'buffer: 'frame,
        Self: 'frame,
    {
        frame.as_mut()
    }

    fn lift(err: GlesError) -> Self::Error {
        MultiError::Render(err)
    }
}

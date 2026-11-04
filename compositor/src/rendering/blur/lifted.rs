//! The blurred scene a backdrop lifts for the surface laid on it.
//!
//! A surface drawn over its own glass (`composited_blur`) is one draw, but its
//! glass still has to be lifted out of the frame where its backdrop always
//! stood: after everything under the surface, and before the shadow it casts,
//! which must not be blurred into its own glass. So the backdrop keeps its
//! place in the list and its damage, lifts and blurs exactly as it always
//! did, and hands the result over instead of drawing it.

use std::{cell::RefCell, rc::Rc};

use smithay::backend::renderer::gles::{Uniform, ffi};

use crate::shaders::lock_glyph::SCENE_UNIT;

/// The pyramid level a backdrop blurred this draw, and the uniforms its own
/// last upsample would have drawn it with.
#[derive(Debug, Clone)]
pub struct LiftedGlass {
    pub texture: ffi::types::GLuint,
    pub uniforms: [Uniform<'static>; 15],
}

/// One frame's hand-over from a backdrop to the surface laid on it. Empty
/// until the backdrop draws, and emptied again when the surface does, so a
/// surface drawn where its backdrop was not never reads a stale scene.
#[derive(Debug, Clone, Default)]
pub struct GlassHandoff(Rc<RefCell<Option<LiftedGlass>>>);

impl GlassHandoff {
    pub(super) fn hand_over(&self, lifted: LiftedGlass) {
        *self.0.borrow_mut() = Some(lifted);
    }

    pub fn take(&self) -> Option<LiftedGlass> {
        self.0.borrow_mut().take()
    }
}

/// Binds `texture` on the unit glass read the scene from, filtered, and leaves
/// unit 0 active for the renderer. Zero unbinds it.
///
/// # Safety
///
/// `gl` must belong to the frame's context.
pub unsafe fn bind_scene(gl: &ffi::Gles2, texture: ffi::types::GLuint) {
    unsafe {
        gl.ActiveTexture(ffi::TEXTURE0 + SCENE_UNIT as u32);
        gl.BindTexture(ffi::TEXTURE_2D, texture);
        if texture != 0 {
            let linear = ffi::LINEAR as i32;
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, linear);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, linear);
        }
        gl.ActiveTexture(ffi::TEXTURE0);
    }
}

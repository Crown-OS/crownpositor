//! The raw GL program behind one dual-kawase pass.
//!
//! Not a smithay custom texture shader: a pyramid pass covers a whole offscreen
//! level with one attribute-less triangle and reads its position from
//! `gl_FragCoord`, which smithay's instanced-quad texture machinery — built to
//! place a texture inside an output — cannot express.

use std::ffi::CStr;

use smithay::backend::renderer::gles::{GlesError, GlesTexture, ffi};

use crate::shaders::program;

static VERTEX_SHADER: &str = include_str!("./kawase.vert");

/// One compiled pass and the uniform slots it is driven through.
#[derive(Debug, Clone, Copy)]
pub struct KawaseProgram {
    program: ffi::types::GLuint,
    tex: ffi::types::GLint,
    destination_size: ffi::types::GLint,
    half_pixel: ffi::types::GLint,
    offset: ffi::types::GLint,
}

impl KawaseProgram {
    /// # Safety
    ///
    /// `gl` must belong to the current context.
    pub unsafe fn compile(gl: &ffi::Gles2, fragment: &str) -> Result<Self, GlesError> {
        unsafe {
            let program = program::link(gl, VERTEX_SHADER, fragment)?;

            let location = |name: &CStr| gl.GetUniformLocation(program, name.as_ptr());
            Ok(Self {
                program,
                tex: location(c"tex"),
                destination_size: location(c"destination_size"),
                half_pixel: location(c"half_pixel"),
                offset: location(c"offset"),
            })
        }
    }

    /// Makes this pass current, reading `source`. Each [`Self::draw`] after
    /// it then fills whatever the scissor leaves of the attached framebuffer.
    ///
    /// # Safety
    ///
    /// `gl` must belong to the current context, with a complete draw
    /// framebuffer bound and the viewport covering `destination_size`.
    pub unsafe fn bind(
        &self,
        gl: &ffi::Gles2,
        source: &GlesTexture,
        destination_size: (f32, f32),
        half_pixel: (f32, f32),
        offset: f32,
    ) {
        unsafe {
            gl.UseProgram(self.program);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(ffi::TEXTURE_2D, source.tex_id());
            // Smithay's offscreen textures carry no sampler state, and the
            // default minification filter leaves them incomplete.
            for (parameter, value) in [
                (ffi::TEXTURE_MIN_FILTER, ffi::LINEAR),
                (ffi::TEXTURE_MAG_FILTER, ffi::LINEAR),
                (ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE),
                (ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE),
            ] {
                gl.TexParameteri(ffi::TEXTURE_2D, parameter, value as ffi::types::GLint);
            }

            gl.Uniform1i(self.tex, 0);
            gl.Uniform2f(
                self.destination_size,
                destination_size.0,
                destination_size.1,
            );
            gl.Uniform2f(self.half_pixel, half_pixel.0, half_pixel.1);
            gl.Uniform1f(self.offset, offset);
        }
    }

    /// # Safety
    ///
    /// A [`Self::bind`] must be current.
    pub unsafe fn draw(gl: &ffi::Gles2) {
        unsafe { gl.DrawArrays(ffi::TRIANGLES, 0, 3) };
    }
}

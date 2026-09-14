//! The raw GL program behind one dual-kawase pass.
//!
//! Not a smithay custom texture shader: a pyramid pass covers a whole offscreen
//! level with one attribute-less triangle and reads its position from
//! `gl_FragCoord`, which smithay's instanced-quad texture machinery — built to
//! place a texture inside an output — cannot express.

use std::ffi::CStr;

use smithay::backend::renderer::gles::{GlesError, GlesTexture, ffi};

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
            let vertex = compile_stage(gl, ffi::VERTEX_SHADER, VERTEX_SHADER)?;
            let fragment = compile_stage(gl, ffi::FRAGMENT_SHADER, fragment)?;

            let program = gl.CreateProgram();
            if program == 0 {
                gl.DeleteShader(vertex);
                gl.DeleteShader(fragment);
                return Err(GlesError::CreateShaderObject);
            }

            gl.AttachShader(program, vertex);
            gl.AttachShader(program, fragment);
            gl.LinkProgram(program);
            gl.DetachShader(program, vertex);
            gl.DetachShader(program, fragment);
            gl.DeleteShader(vertex);
            gl.DeleteShader(fragment);

            let mut linked = 0;
            gl.GetProgramiv(program, ffi::LINK_STATUS, &mut linked);
            if linked == 0 {
                gl.DeleteProgram(program);
                return Err(GlesError::ProgramLinkError);
            }

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

    /// Fills the currently attached framebuffer from `source`.
    ///
    /// # Safety
    ///
    /// `gl` must belong to the current context, with a complete draw
    /// framebuffer bound and the viewport covering `destination_size`.
    pub unsafe fn run(
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
            gl.DrawArrays(ffi::TRIANGLES, 0, 3);
        }
    }
}

unsafe fn compile_stage(
    gl: &ffi::Gles2,
    stage: ffi::types::GLenum,
    source: &str,
) -> Result<ffi::types::GLuint, GlesError> {
    unsafe {
        let shader = gl.CreateShader(stage);
        if shader == 0 {
            return Err(GlesError::CreateShaderObject);
        }

        let pointer = source.as_ptr().cast::<ffi::types::GLchar>();
        let length = source.len() as ffi::types::GLint;
        gl.ShaderSource(shader, 1, &pointer, &length);
        gl.CompileShader(shader);

        let mut compiled = 0;
        gl.GetShaderiv(shader, ffi::COMPILE_STATUS, &mut compiled);
        if compiled == 0 {
            gl.DeleteShader(shader);
            return Err(GlesError::ShaderCompileError);
        }
        Ok(shader)
    }
}

//! Compiling and linking the raw GL programs that bypass smithay's
//! texture-shader machinery: the blur pyramid and the window warps.

use smithay::backend::renderer::gles::{GlesError, ffi};

/// Links a vertex and a fragment stage into a program.
///
/// # Safety
///
/// `gl` must belong to the current context.
pub unsafe fn link(
    gl: &ffi::Gles2,
    vertex: &str,
    fragment: &str,
) -> Result<ffi::types::GLuint, GlesError> {
    unsafe {
        let vertex = compile_stage(gl, ffi::VERTEX_SHADER, vertex)?;
        let fragment = match compile_stage(gl, ffi::FRAGMENT_SHADER, fragment) {
            Ok(fragment) => fragment,
            Err(err) => {
                gl.DeleteShader(vertex);
                return Err(err);
            }
        };

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
        Ok(program)
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
            let mut log = [0u8; 1024];
            let mut written = 0;
            gl.GetShaderInfoLog(
                shader,
                log.len() as ffi::types::GLsizei,
                &mut written,
                log.as_mut_ptr().cast(),
            );
            let written = usize::try_from(written).unwrap_or(0).min(log.len());
            tracing::warn!(log = %String::from_utf8_lossy(&log[..written]), "shader failed to compile");
            gl.DeleteShader(shader);
            return Err(GlesError::ShaderCompileError);
        }
        Ok(shader)
    }
}

//! The window-border shader program.
//!
//! A *pixel* program rather than a texture one — see `border.frag` — so it is
//! driven through [`GlesFrame::render_pixel_shader_to`] and needs no buffer to
//! sample. Otherwise it follows the same shape as the other two: compiled once
//! per GL context, parked in the EGL context's user data so it is reachable
//! anywhere the renderer is, and fetched through a fallible [`get`] so a
//! compile failure costs the border rather than the compositor.
//!
//! [`GlesFrame::render_pixel_shader_to`]: smithay::backend::renderer::gles::GlesFrame::render_pixel_shader_to
//! [`get`]: BorderShader::get

use smithay::backend::renderer::gles::{
    GlesError, GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType,
};

static BORDER_SHADER: &str = include_str!("./border.frag");

/// The compiled border program for one GL context.
#[derive(Debug, Clone)]
pub struct BorderShader(pub GlesPixelProgram);

impl BorderShader {
    /// `size`, `alpha` and `tint` are *not* here: the renderer supplies those
    /// to every pixel program, and naming them again would ask GL for a uniform
    /// location twice.
    fn uniforms() -> [UniformName<'static>; 3] {
        [
            UniformName::new("color", UniformType::_4f),
            UniformName::new("thickness", UniformType::_1f),
            UniformName::new("radius", UniformType::_1f),
        ]
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program = renderer.compile_custom_pixel_shader(BORDER_SHADER, &Self::uniforms())?;

        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| BorderShader(program));

        Ok(())
    }

    /// `None` when `init` was never called (or failed): windows then draw
    /// without a border rather than taking the compositor down mid-frame.
    pub fn get(renderer: &GlesRenderer) -> Option<GlesPixelProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<BorderShader>()
            .map(|shader| shader.0.clone())
    }

    /// Uniforms for one ring. `color` is premultiplied, `thickness` and
    /// `radius` are physical pixels, and `radius` is the *outer* one.
    pub fn values(color: [f32; 4], thickness: f32, radius: f32) -> [Uniform<'static>; 3] {
        [
            Uniform::new("color", color),
            Uniform::new("thickness", thickness),
            Uniform::new("radius", radius),
        ]
    }
}

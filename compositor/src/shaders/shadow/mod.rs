//! The drop-shadow shader program.
//!
//! The same shape as [`BorderShader`]: a pixel program compiled once per GL
//! context, parked in the EGL context's user data so it is reachable anywhere
//! the renderer is, and fetched through a fallible [`get`] so a compile failure
//! costs the shadow rather than the compositor.
//!
//! [`BorderShader`]: crate::shaders::border::BorderShader
//! [`get`]: ShadowShader::get

use smithay::{
    backend::renderer::gles::{
        GlesError, GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType,
    },
    utils::{Physical, Rectangle},
};

static SHADOW_SHADER: &str = include_str!("./shadow.frag");

/// The compiled shadow program for one GL context.
#[derive(Debug, Clone)]
pub struct ShadowShader(pub GlesPixelProgram);

impl ShadowShader {
    /// `size`, `alpha` and `tint` are *not* here: the renderer supplies those
    /// to every pixel program, and naming them again would ask GL for a uniform
    /// location twice.
    fn uniforms() -> [UniformName<'static>; 5] {
        [
            UniformName::new("color", UniformType::_4f),
            UniformName::new("shape_origin", UniformType::_2f),
            UniformName::new("shape_size", UniformType::_2f),
            UniformName::new("shape_radius", UniformType::_1f),
            UniformName::new("sigma", UniformType::_1f),
        ]
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program = renderer.compile_custom_pixel_shader(SHADOW_SHADER, &Self::uniforms())?;

        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| ShadowShader(program));

        Ok(())
    }

    /// `None` when `init` was never called (or failed): windows then draw
    /// without a shadow rather than taking the compositor down mid-frame.
    pub fn get(renderer: &GlesRenderer) -> Option<GlesPixelProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<ShadowShader>()
            .map(|shader| shader.0.clone())
    }

    /// Uniforms for one shadow. `color` is premultiplied, and `shape` is the
    /// silhouette's rect *relative to the element*, which is the space
    /// `v_coords * size` lands in.
    pub fn values(
        color: [f32; 4],
        shape: Rectangle<i32, Physical>,
        radius: f32,
        sigma: f32,
    ) -> [Uniform<'static>; 5] {
        [
            Uniform::new("color", color),
            Uniform::new("shape_origin", (shape.loc.x as f32, shape.loc.y as f32)),
            Uniform::new("shape_size", (shape.size.w as f32, shape.size.h as f32)),
            Uniform::new("shape_radius", radius),
            Uniform::new("sigma", sigma),
        ]
    }
}

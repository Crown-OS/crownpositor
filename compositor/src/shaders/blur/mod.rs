//! The blur pipeline's shader programs.
//!
//! Three texture programs, all driven through smithay's custom-texture-shader
//! machinery so they inherit its vertex stage, damage instancing and variant
//! handling (`EXTERNAL`/`NO_ALPHA`/`DEBUG_FLAGS`):
//!
//! * `down` / `up` — the dual-kawase pyramid passes. Run offscreen by
//!   [`rendering::blur`], never against client buffers.
//! * `finish` — samples the blurred scene under one window, dithers it and
//!   masks the same rounded corners the window is drawn with.
//!
//! Like [`RoundedCornerShader`], the compiled programs live in the EGL
//! context's user data: per GPU, reachable from anywhere the renderer is.
//!
//! [`rendering::blur`]: crate::rendering::blur
//! [`RoundedCornerShader`]: crate::shaders::rounded_corner::RoundedCornerShader

use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};

static DOWN_SHADER: &str = include_str!("./blur_down.frag");
static UP_SHADER: &str = include_str!("./blur_up.frag");
static FINISH_SHADER: &str = include_str!("./blur_finish.frag");

/// The compiled blur programs for one GL context.
#[derive(Debug, Clone)]
pub struct BlurShaders {
    pub down: GlesTexProgram,
    pub up: GlesTexProgram,
    pub finish: GlesTexProgram,
}

impl BlurShaders {
    fn kawase_uniforms() -> [UniformName<'static>; 2] {
        [
            UniformName::new("half_pixel", UniformType::_2f),
            UniformName::new("offset", UniformType::_1f),
        ]
    }

    fn finish_uniforms() -> [UniformName<'static>; 3] {
        [
            UniformName::new("geo_size", UniformType::_2f),
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("noise", UniformType::_1f),
        ]
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let down = renderer.compile_custom_texture_shader(DOWN_SHADER, &Self::kawase_uniforms())?;
        let up = renderer.compile_custom_texture_shader(UP_SHADER, &Self::kawase_uniforms())?;
        let finish =
            renderer.compile_custom_texture_shader(FINISH_SHADER, &Self::finish_uniforms())?;

        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| BlurShaders { down, up, finish });

        Ok(())
    }

    /// `None` when `init` was never called (or failed): blur then degrades to
    /// windows without backdrops rather than taking the compositor down.
    pub fn get(renderer: &GlesRenderer) -> Option<BlurShaders> {
        renderer
            .egl_context()
            .user_data()
            .get::<BlurShaders>()
            .cloned()
    }

    /// Uniforms for one kawase pass. `half_pixel` is half a pixel of the
    /// *smaller* texture involved in the pass (destination when downsampling,
    /// source when upsampling), in UV space.
    pub fn kawase_values(half_pixel: (f32, f32), offset: f32) -> [Uniform<'static>; 2] {
        [
            Uniform::new("half_pixel", half_pixel),
            Uniform::new("offset", offset),
        ]
    }

    pub fn finish_values(
        geo_size: (f32, f32),
        corner_radius: f32,
        noise: f32,
    ) -> [Uniform<'static>; 3] {
        [
            Uniform::new("geo_size", geo_size),
            Uniform::new("corner_radius", corner_radius),
            Uniform::new("noise", noise),
        ]
    }
}

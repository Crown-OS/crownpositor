use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};

pub static SPRITE_BLUR_SHADER: &str = include_str!("./sprite_blur.frag");

/// Blurs a compositor-drawn sprite in place of the default texture program:
/// what a lock-screen layer blurring in or out is drawn with.
pub struct SpriteBlurShader(pub GlesTexProgram);

impl SpriteBlurShader {
    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program = renderer.compile_custom_texture_shader(
            SPRITE_BLUR_SHADER,
            &[UniformName::new("reach", UniformType::_2f)],
        )?;
        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| SpriteBlurShader(program));
        Ok(())
    }

    pub fn get(renderer: &GlesRenderer) -> Option<GlesTexProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<SpriteBlurShader>()
            .map(|shader| shader.0.clone())
    }

    /// `radius` in texels of a texture `size` texels big.
    pub fn uniform_values(radius: f32, size: [f64; 2]) -> [Uniform<'static>; 1] {
        let reach = size.map(|side| {
            if side > 0.0 {
                radius / side as f32
            } else {
                0.0
            }
        });
        [Uniform::new("reach", (reach[0], reach[1]))]
    }
}

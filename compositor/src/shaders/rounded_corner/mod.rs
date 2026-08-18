use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};

pub static CLIPPING_SHADER: &str = include_str!("./rounded_corner.frag");
pub struct RoundedCornerShader(pub GlesTexProgram);

impl RoundedCornerShader {
    fn uniforms() -> [UniformName<'static>; 2] {
        [
            UniformName::new("size", UniformType::_2f),
            UniformName::new("radius", UniformType::_1f),
        ]
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program = renderer.compile_custom_texture_shader(CLIPPING_SHADER, &Self::uniforms())?;

        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| RoundedCornerShader(program));

        Ok(())
    }

    pub fn get(renderer: &GlesRenderer) -> GlesTexProgram {
        renderer
            .egl_context()
            .user_data()
            .get::<RoundedCornerShader>()
            .expect("Custom shaders not initialized")
            .0
            .clone()
    }

    pub fn uniform_values(size: (f32, f32), radius: f32) -> [Uniform<'static>; 2] {
        [
            Uniform::new("size", (size.0, size.1)),
            Uniform::new("radius", radius),
        ]
    }
}

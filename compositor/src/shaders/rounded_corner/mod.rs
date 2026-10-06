use smithay::{
    backend::renderer::gles::{
        GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
    },
    utils::{Physical, Rectangle},
};

pub static CLIPPING_SHADER: &str = concat!(
    include_str!("./rounded_corner.frag"),
    include_str!("../common/rounded_box.glsl"),
);
pub struct RoundedCornerShader(pub GlesTexProgram);

impl RoundedCornerShader {
    fn uniforms() -> [UniformName<'static>; 3] {
        [
            UniformName::new("shape_origin", UniformType::_2f),
            UniformName::new("shape_size", UniformType::_2f),
            UniformName::new("radius", UniformType::_4f),
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

    /// `None` when `init` was never called, so a missing shader degrades to
    /// square corners rather than taking the compositor down mid-frame.
    pub fn get(renderer: &GlesRenderer) -> Option<GlesTexProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<RoundedCornerShader>()
            .map(|shader| shader.0.clone())
    }

    /// `shape` and `radius` are in framebuffer pixels, the radii per
    /// framebuffer corner: (+x, +y), (+x, -y), (-x, +y), (-x, -y).
    pub fn uniform_values(
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
    ) -> [Uniform<'static>; 3] {
        [
            Uniform::new("shape_origin", (shape.loc.x as f32, shape.loc.y as f32)),
            Uniform::new("shape_size", (shape.size.w as f32, shape.size.h as f32)),
            Uniform::new("radius", radius),
        ]
    }
}

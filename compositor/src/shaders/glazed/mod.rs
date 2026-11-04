//! The program a composited surface is drawn with: its own pixels over its
//! glass, cut by the shape's edge once. See `glazed.frag`.

use smithay::{
    backend::renderer::gles::{
        GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
    },
    utils::{Physical, Rectangle},
};

use crate::shaders::{
    blur::BlurShaders, lock_glyph::SCENE_UNIT, rounded_corner::RoundedCornerShader,
};

static GLAZED_SHADER: &str = concat!(
    include_str!("./glazed.frag"),
    include_str!("../common/rounded_box.glsl"),
    include_str!("../common/glass.glsl"),
);

pub struct GlazedShader(pub GlesTexProgram);

impl GlazedShader {
    /// The rounded clip's uniforms, the glass's, and the scene's unit and
    /// switch: everything both programs it combines take.
    fn uniforms() -> Vec<UniformName<'static>> {
        RoundedCornerShader::uniforms()
            .into_iter()
            .chain(BlurShaders::finish_names())
            .chain([
                UniformName::new("scene_tex", UniformType::_1i),
                UniformName::new("glazed", UniformType::_1f),
            ])
            .collect()
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program = renderer.compile_custom_texture_shader(GLAZED_SHADER, &Self::uniforms())?;
        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| GlazedShader(program));
        Ok(())
    }

    /// `None` when `init` never ran or failed; the surface is then drawn and
    /// backed the way a version 1 client's is.
    pub fn get(renderer: &GlesRenderer) -> Option<GlesTexProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<GlazedShader>()
            .map(|shader| shader.0.clone())
    }

    /// The shape a surface with no glass under it is still cut by, in
    /// framebuffer pixels.
    pub fn cut_values(mask: Rectangle<i32, Physical>, corner_radius: f32) -> [Uniform<'static>; 3] {
        [
            Uniform::new("mask_origin", (mask.loc.x as f32, mask.loc.y as f32)),
            Uniform::new("mask_size", (mask.size.w as f32, mask.size.h as f32)),
            Uniform::new("corner_radius", corner_radius),
        ]
    }

    /// Where the scene is read from, and whether it holds glass for this draw.
    /// The unit is set either way: left at zero, the sampler would share the
    /// surface's unit, which GL rejects when that one is an external texture.
    pub fn scene_values(glazed: bool) -> [Uniform<'static>; 2] {
        [
            Uniform::new("scene_tex", SCENE_UNIT),
            Uniform::new("glazed", if glazed { 1.0f32 } else { 0.0 }),
        ]
    }
}

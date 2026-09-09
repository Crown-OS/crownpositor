//! The titlebar shader program.
//!
//! A *pixel* program, like [`BorderShader`]: the titlebar samples no buffer,
//! it generates its panel, its highlight and its three controls from their
//! geometry. Compiled once per GL context, parked in the EGL context's user
//! data, and fetched through a fallible [`get`] so a compile failure costs the
//! decoration rather than the compositor.
//!
//! [`BorderShader`]: crate::shaders::border::BorderShader
//! [`get`]: TitleBarShader::get

use smithay::backend::renderer::gles::{
    GlesError, GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType,
};

static TITLE_BAR_SHADER: &str = include_str!("./title_bar.frag");

/// Everything one titlebar's pixels are generated from.
///
/// Physical pixels throughout, relative to the element's own origin — the
/// shader works in the same space `v_coords * size` puts it in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitleBarUniforms {
    /// The sheet's rect relative to the element's own origin, which everything
    /// is masked by — so the tint traces the window's curve wherever the client
    /// does not cover it.
    pub window_offset: (f32, f32),
    pub window_size: (f32, f32),
    pub radius: f32,
    /// Height of the titlebar band, down from the sheet's top edge. The
    /// gradient runs over this, not over the whole sheet.
    pub bar_height: f32,
    /// Premultiplied gradient endpoints, top to bottom.
    pub tint_top: [f32; 4],
    pub tint_bottom: [f32; 4],
    pub highlight: [f32; 4],
    pub control_origin: (f32, f32),
    pub control_spacing: f32,
    pub control_radius: f32,
    pub control_fill: [f32; 4],
    pub control_glyph: [f32; 4],
    /// Index of the control under the pointer; anything outside `0..3` is none.
    pub control_hover: f32,
}

/// The compiled titlebar program for one GL context.
#[derive(Debug, Clone)]
pub struct TitleBarShader(pub GlesPixelProgram);

impl TitleBarShader {
    /// `size`, `alpha` and `tint` are absent on purpose: the renderer supplies
    /// those to every pixel program, and naming them again would ask GL for a
    /// uniform location twice.
    fn uniforms() -> [UniformName<'static>; 13] {
        [
            UniformName::new("window_offset", UniformType::_2f),
            UniformName::new("window_size", UniformType::_2f),
            UniformName::new("radius", UniformType::_1f),
            UniformName::new("bar_height", UniformType::_1f),
            UniformName::new("tint_top", UniformType::_4f),
            UniformName::new("tint_bottom", UniformType::_4f),
            UniformName::new("highlight", UniformType::_4f),
            UniformName::new("control_origin", UniformType::_2f),
            UniformName::new("control_spacing", UniformType::_1f),
            UniformName::new("control_radius", UniformType::_1f),
            UniformName::new("control_fill", UniformType::_4f),
            UniformName::new("control_glyph", UniformType::_4f),
            UniformName::new("control_hover", UniformType::_1f),
        ]
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program = renderer.compile_custom_pixel_shader(TITLE_BAR_SHADER, &Self::uniforms())?;

        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| TitleBarShader(program));

        Ok(())
    }

    /// `None` when `init` was never called or failed: windows then draw
    /// without a frame rather than taking the compositor down mid-frame.
    pub fn get(renderer: &GlesRenderer) -> Option<GlesPixelProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<TitleBarShader>()
            .map(|shader| shader.0.clone())
    }

    pub fn values(uniforms: &TitleBarUniforms) -> [Uniform<'static>; 13] {
        [
            Uniform::new("window_offset", uniforms.window_offset),
            Uniform::new("window_size", uniforms.window_size),
            Uniform::new("radius", uniforms.radius),
            Uniform::new("bar_height", uniforms.bar_height),
            Uniform::new("tint_top", uniforms.tint_top),
            Uniform::new("tint_bottom", uniforms.tint_bottom),
            Uniform::new("highlight", uniforms.highlight),
            Uniform::new("control_origin", uniforms.control_origin),
            Uniform::new("control_spacing", uniforms.control_spacing),
            Uniform::new("control_radius", uniforms.control_radius),
            Uniform::new("control_fill", uniforms.control_fill),
            Uniform::new("control_glyph", uniforms.control_glyph),
            Uniform::new("control_hover", uniforms.control_hover),
        ]
    }
}

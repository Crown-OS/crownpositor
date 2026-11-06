use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType, UniformValue,
};

use lockscreen::clock::{RIM_WHITE, TOP_SHINE_REACH};

use crate::rendering::decorate::GlyphDraw;

pub static LOCK_GLYPH_SHADER: &str = include_str!("./lock_glyph.frag");
const DEFINES: &str = "//_DEFINES_";

/// Draws one lock-screen clock glyph bent by the clock's warp, blurred while
/// it changes or its layer moves; the glass program fills it with frosted
/// glass.
#[derive(Clone)]
pub struct LockGlyphShader {
    pub plain: GlesTexProgram,
    pub glass: GlesTexProgram,
}

/// The pyramid level a glass glyph samples, and how.
#[derive(Debug, Clone, Copy)]
pub struct Frost {
    pub scene: [f32; 2],
    pub half_pixel: (f32, f32),
    pub offset: f32,
}

/// The texture unit glass glyphs read the pyramid from.
pub const SCENE_UNIT: i32 = 1;

impl LockGlyphShader {
    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let shape = [
            UniformName::new("unwarp", UniformType::Matrix3x3),
            UniformName::new("reach", UniformType::_2f),
        ];
        let plain = renderer.compile_custom_texture_shader(LOCK_GLYPH_SHADER, &shape)?;
        let glass_source = LOCK_GLYPH_SHADER.replacen(DEFINES, "//_DEFINES_\n#define GLASS", 1);
        let glass_names: Vec<_> = shape
            .into_iter()
            .chain([
                UniformName::new("scene_tex", UniformType::_1i),
                UniformName::new("scene_size", UniformType::_2f),
                UniformName::new("half_pixel", UniformType::_2f),
                UniformName::new("offset", UniformType::_1f),
                UniformName::new("gain", UniformType::_1f),
                UniformName::new("white", UniformType::_1f),
                UniformName::new("top_shine", UniformType::_1f),
                UniformName::new("top_shine_reach", UniformType::_1f),
                UniformName::new("rim_white", UniformType::_1f),
                UniformName::new("rows", UniformType::_2f),
                UniformName::new("rim_reach", UniformType::_2f),
            ])
            .collect();
        let glass = renderer.compile_custom_texture_shader(&glass_source, &glass_names)?;
        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| LockGlyphShader { plain, glass });
        Ok(())
    }

    pub fn get(renderer: &GlesRenderer) -> Option<LockGlyphShader> {
        renderer
            .egl_context()
            .user_data()
            .get::<LockGlyphShader>()
            .cloned()
    }

    pub fn uniform_values(glyph: &GlyphDraw) -> Vec<Uniform<'static>> {
        vec![
            Uniform::new(
                "unwarp",
                UniformValue::Matrix3x3 {
                    matrices: vec![glyph.unwarp],
                    transpose: false,
                },
            ),
            Uniform::new("reach", (glyph.reach[0], glyph.reach[1])),
        ]
    }

    /// What the glass program needs beyond [`uniform_values`](Self::uniform_values).
    pub fn glass_values(glyph: &GlyphDraw, frost: Frost) -> Vec<Uniform<'static>> {
        let Some(glass) = &glyph.glass else {
            return Vec::new();
        };
        vec![
            Uniform::new("scene_tex", SCENE_UNIT),
            Uniform::new("scene_size", (frost.scene[0], frost.scene[1])),
            Uniform::new("half_pixel", frost.half_pixel),
            Uniform::new("offset", frost.offset),
            Uniform::new("gain", glass.face.gain * glass.frost_gain),
            Uniform::new("white", glass.face.white),
            Uniform::new("top_shine", glass.face.top_shine),
            Uniform::new("top_shine_reach", TOP_SHINE_REACH),
            Uniform::new("rim_white", RIM_WHITE),
            Uniform::new("rows", (glass.rows[0], glass.rows[1])),
            Uniform::new("rim_reach", (glass.rim[0], glass.rim[1])),
        ]
    }
}

#[cfg(test)]
mod tests {
    use lockscreen::clock::rim;

    use super::*;

    /// How the shader erodes the glyph and lights its rim, line for line.
    const SHADER_RIM: [&str; 5] = [
        "glyph(uv + vec2(rim_reach.x, 0.0)).a",
        "glyph(uv - vec2(rim_reach.x, 0.0)).a",
        "glyph(uv + vec2(0.0, rim_reach.y)).a",
        "glyph(uv - vec2(0.0, rim_reach.y)).a",
        "float rim = clamp(coverage - eroded, 0.0, 1.0);",
    ];

    fn shader_rim(coverage: f32, eroded: f32) -> f32 {
        (coverage - eroded).clamp(0.0, 1.0)
    }

    #[test]
    fn the_shader_lights_the_rim_as_the_preview_does() {
        for line in SHADER_RIM {
            assert!(LOCK_GLYPH_SHADER.contains(line), "{line}");
        }
        for (coverage, eroded) in [(1.0, 0.0), (0.6, 0.2), (0.3, 0.9), (1.0, 1.0)] {
            assert_eq!(rim(coverage, eroded), shader_rim(coverage, eroded));
        }
    }
}

use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType, UniformValue,
};

use crate::color::pipeline::SurfaceTransform;

/// The rounded-corner shader with colour decoding fused in.
///
/// Fused rather than run as a second pass: `override_default_tex_program` is
/// one program per draw, so a separate colour pass would mean drawing every
/// window twice.
pub static COLOR_WINDOW_SHADER: &str = concat!(
    include_str!("./color_window.frag"),
    include_str!("../common/rounded_box.glsl"),
);

pub struct ColorWindowShader(pub GlesTexProgram);

impl ColorWindowShader {
    fn uniforms() -> [UniformName<'static>; 6] {
        [
            UniformName::new("size", UniformType::_2f),
            UniformName::new("radius", UniformType::_4f),
            UniformName::new("tf_kind", UniformType::_1i),
            UniformName::new("tf_param", UniformType::_1f),
            UniformName::new("luminance_scale", UniformType::_1f),
            UniformName::new("primaries", UniformType::Matrix3x3),
        ]
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program =
            renderer.compile_custom_texture_shader(COLOR_WINDOW_SHADER, &Self::uniforms())?;

        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| ColorWindowShader(program));

        Ok(())
    }

    /// `None` when the shader never compiled, which makes the caller fall back
    /// to the plain rounded-corner program — wrong colours beat no window.
    pub fn get(renderer: &GlesRenderer) -> Option<GlesTexProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<ColorWindowShader>()
            .map(|shader| shader.0.clone())
    }

    pub fn uniform_values(
        size: (f32, f32),
        radius: [f32; 4],
        transform: SurfaceTransform,
    ) -> [Uniform<'static>; 6] {
        [
            Uniform::new("size", (size.0, size.1)),
            Uniform::new("radius", radius),
            Uniform::new("tf_kind", transform.transfer as i32),
            Uniform::new("tf_param", transform.transfer_param),
            Uniform::new("luminance_scale", transform.luminance_scale),
            Uniform::new(
                "primaries",
                // No `From<[f32; 9]>` exists, so the variant is built directly.
                UniformValue::Matrix3x3 {
                    matrices: vec![flatten(transform.primaries)],
                    transpose: false,
                },
            ),
        ]
    }
}

/// Column-major, which is what `UniformMatrix3fv` reads without transposing.
fn flatten(matrix: [[f32; 3]; 3]) -> [f32; 9] {
    let mut out = [0.0; 9];
    for (column, values) in matrix.iter().enumerate() {
        out[column * 3..column * 3 + 3].copy_from_slice(values);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matrix_is_flattened_column_by_column() {
        let matrix = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
        assert_eq!(
            flatten(matrix),
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]
        );
    }

    #[test]
    fn both_programs_share_one_copy_of_the_distance_field() {
        use crate::shaders::rounded_corner::CLIPPING_SHADER;

        let body = include_str!("../common/rounded_box.glsl");
        assert!(COLOR_WINDOW_SHADER.ends_with(body));
        assert!(CLIPPING_SHADER.ends_with(body));

        // And each declares the prototype, without which GLSL ES 1.00 would
        // reject the call that comes before the body.
        for shader in [COLOR_WINDOW_SHADER, CLIPPING_SHADER] {
            assert!(
                shader.contains("float rounded_box(in vec2 p, in vec2 b, in vec4 r);"),
                "the prototype must precede the call"
            );
        }
    }

    #[test]
    fn the_shader_defines_match_the_rust_enum() {
        use crate::color::pipeline::TransferKind;

        // The two lists are written out separately on purpose — the shader
        // cannot see the enum — so this is what keeps them honest.
        for (kind, define) in [
            (TransferKind::Linear, "#define TF_LINEAR         0"),
            (TransferKind::Power, "#define TF_POWER          1"),
            (TransferKind::SrgbPiecewise, "#define TF_SRGB_PIECEWISE 2"),
            (TransferKind::SrgbExtended, "#define TF_SRGB_EXTENDED  3"),
            (TransferKind::St240, "#define TF_ST240          4"),
            (TransferKind::St2084Pq, "#define TF_ST2084_PQ      5"),
            (TransferKind::Hlg, "#define TF_HLG            6"),
            (TransferKind::St428, "#define TF_ST428          7"),
        ] {
            assert!(
                COLOR_WINDOW_SHADER.contains(define),
                "{kind:?} is missing its `{define}`"
            );
            let value = define
                .rsplit(' ')
                .next()
                .and_then(|value| value.parse::<i32>().ok())
                .expect("the define ends in its value");
            assert_eq!(kind as i32, value, "{kind:?} disagrees with the shader");
        }
    }
}

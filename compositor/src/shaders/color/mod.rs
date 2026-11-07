use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType, UniformValue,
};

use crate::{
    color::pipeline::SurfaceTransform,
    shaders::rounded_corner::{CLIPPING_SHADER, RoundedCornerShader},
};

const DEFINES_MARKER: &str = "//_DEFINES_";

/// The rounded-corner program with colour conversion fused in.
///
/// Fused rather than run as a second pass: `override_default_tex_program` is
/// one program per draw, so a separate colour pass would mean drawing every
/// described window twice. It keeps the corner clip and the area filter, so a
/// converted window is shaped and sampled exactly like any other.
pub fn color_convert_source() -> String {
    let converting = CLIPPING_SHADER.replacen(
        DEFINES_MARKER,
        &format!("#define COLOR_CONVERT\n{DEFINES_MARKER}"),
        1,
    );
    converting + include_str!("./color_convert.glsl")
}

pub struct ColorConvertShader(pub GlesTexProgram);

impl ColorConvertShader {
    fn uniforms() -> Vec<UniformName<'static>> {
        let mut uniforms = RoundedCornerShader::uniforms().to_vec();
        uniforms.extend([
            UniformName::new("tf_kind", UniformType::_1i),
            UniformName::new("tf_param", UniformType::_1f),
            UniformName::new("luminance_scale", UniformType::_1f),
            UniformName::new("headroom", UniformType::_1f),
            UniformName::new("primaries", UniformType::Matrix3x3),
        ]);
        uniforms
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program =
            renderer.compile_custom_texture_shader(color_convert_source(), &Self::uniforms())?;

        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| ColorConvertShader(program));

        Ok(())
    }

    /// `None` when the shader never compiled, which makes the caller fall back
    /// to the plain rounded-corner program — wrong colours beat no window.
    pub fn get(renderer: &GlesRenderer) -> Option<GlesTexProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<ColorConvertShader>()
            .map(|shader| shader.0.clone())
    }

    /// The uniforms on top of [`RoundedCornerShader::uniform_values`].
    pub fn uniform_values(transform: SurfaceTransform) -> [Uniform<'static>; 5] {
        [
            Uniform::new("tf_kind", transform.transfer as i32),
            Uniform::new("tf_param", transform.transfer_param),
            Uniform::new("luminance_scale", transform.luminance_scale),
            Uniform::new("headroom", transform.headroom),
            Uniform::new(
                "primaries",
                UniformValue::Matrix3x3 {
                    matrices: vec![std::array::from_fn(|index| {
                        transform.primaries[index / 3][index % 3]
                    })],
                    transpose: false,
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{
        pipeline::TransferKind,
        tone_map::{KNEE, OUTPUT_GAMMA},
    };

    #[test]
    fn the_variant_is_switched_on_before_any_code() {
        let source = color_convert_source();
        let mut lines = source.lines();
        assert_eq!(lines.next(), Some("#version 100"));
        let define = source
            .find("#define COLOR_CONVERT")
            .expect("the variant's define");
        let marker = source.find(DEFINES_MARKER).expect("smithay's marker");
        assert!(define < marker, "the define must precede smithay's own");
        assert!(define < source.find("#extension").unwrap_or(usize::MAX));
    }

    #[test]
    fn the_plain_program_never_converts() {
        assert!(!CLIPPING_SHADER.contains("#define COLOR_CONVERT"));
        assert!(!CLIPPING_SHADER.contains("vec4 convert_color(vec4 premultiplied) {"));
    }

    #[test]
    fn the_converter_is_declared_before_use_and_defined_after() {
        let source = color_convert_source();
        let declared = source
            .find("vec4 convert_color(vec4 premultiplied);")
            .expect("a prototype");
        let used = source
            .find("color = convert_color(color);")
            .expect("a call");
        let defined = source
            .find("vec4 convert_color(vec4 premultiplied) {")
            .expect("a body");
        assert!(declared < used && used < defined);
    }

    #[test]
    fn the_shader_constants_match_the_cpu_reference() {
        let source = color_convert_source();
        assert!(source.contains(&format!("const float TONE_KNEE = {KNEE:?};")));
        assert!(source.contains(&format!("const float OUTPUT_GAMMA = {OUTPUT_GAMMA:?};")));
    }

    #[test]
    fn every_extra_uniform_has_a_value() {
        let transform = SurfaceTransform {
            transfer: TransferKind::St2084Pq,
            transfer_param: 1.0,
            luminance_scale: 1.0 / 203.0,
            headroom: 10_000.0 / 203.0,
            primaries: [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]],
        };
        let declared = ColorConvertShader::uniforms();
        let values = ColorConvertShader::uniform_values(transform);
        for value in &values {
            assert!(
                declared.iter().any(|name| name.name == value.name),
                "{} is set but never declared",
                value.name
            );
        }
        assert_eq!(
            declared.len(),
            RoundedCornerShader::uniforms().len() + values.len()
        );
    }

    #[test]
    fn the_matrix_is_flattened_column_by_column() {
        let transform = SurfaceTransform {
            transfer: TransferKind::Linear,
            transfer_param: 1.0,
            luminance_scale: 1.0,
            headroom: 1.0,
            primaries: [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]],
        };
        let [.., matrix] = ColorConvertShader::uniform_values(transform);
        assert_eq!(
            matrix.value,
            UniformValue::Matrix3x3 {
                matrices: vec![[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]],
                transpose: false,
            }
        );
    }

    #[test]
    fn both_programs_share_one_copy_of_the_distance_field() {
        let body = include_str!("../common/rounded_box.glsl");
        assert!(CLIPPING_SHADER.ends_with(body));
        assert_eq!(color_convert_source().matches(body).count(), 1);
    }

    #[test]
    fn the_shader_defines_match_the_rust_enum() {
        let source = color_convert_source();
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
                source.contains(define),
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

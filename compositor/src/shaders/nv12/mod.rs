use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
};

/// RGB → BT.709 limited-range Y'CbCr, for writing NV12 capture buffers on the
/// GPU so a video encoder can take them as they are.
pub static RGB_TO_YCBCR_SHADER: &str = include_str!("./rgb_to_ycbcr.frag");

/// One output channel: `dot(rgb, row) + offset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatrixRow {
    pub row: [f32; 3],
    pub offset: f32,
}

const LUMA_RED: f32 = 0.2126;
const LUMA_BLUE: f32 = 0.0722;
const LUMA_GREEN: f32 = 1.0 - LUMA_RED - LUMA_BLUE;
const LUMA_RANGE: f32 = 219.0 / 255.0;
const CHROMA_RANGE: f32 = 224.0 / 255.0;
const LUMA_FLOOR: f32 = 16.0 / 255.0;
const CHROMA_CENTRE: f32 = 128.0 / 255.0;

pub const LUMA: MatrixRow = MatrixRow {
    row: [
        LUMA_RED * LUMA_RANGE,
        LUMA_GREEN * LUMA_RANGE,
        LUMA_BLUE * LUMA_RANGE,
    ],
    offset: LUMA_FLOOR,
};

const CB_SCALE: f32 = CHROMA_RANGE / (2.0 * (1.0 - LUMA_BLUE));
const CR_SCALE: f32 = CHROMA_RANGE / (2.0 * (1.0 - LUMA_RED));

pub const CHROMA_BLUE: MatrixRow = MatrixRow {
    row: [
        -LUMA_RED * CB_SCALE,
        -LUMA_GREEN * CB_SCALE,
        (1.0 - LUMA_BLUE) * CB_SCALE,
    ],
    offset: CHROMA_CENTRE,
};

pub const CHROMA_RED: MatrixRow = MatrixRow {
    row: [
        (1.0 - LUMA_RED) * CR_SCALE,
        -LUMA_GREEN * CR_SCALE,
        -LUMA_BLUE * CR_SCALE,
    ],
    offset: CHROMA_CENTRE,
};

pub struct Nv12Shader(pub GlesTexProgram);

impl Nv12Shader {
    fn uniforms() -> [UniformName<'static>; 4] {
        [
            UniformName::new("first_row", UniformType::_3f),
            UniformName::new("first_offset", UniformType::_1f),
            UniformName::new("second_row", UniformType::_3f),
            UniformName::new("second_offset", UniformType::_1f),
        ]
    }

    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program =
            renderer.compile_custom_texture_shader(RGB_TO_YCBCR_SHADER, &Self::uniforms())?;
        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| Nv12Shader(program));
        Ok(())
    }

    /// `None` when the shader never compiled, in which case NV12 is simply
    /// not advertised.
    pub fn get(renderer: &GlesRenderer) -> Option<GlesTexProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<Nv12Shader>()
            .map(|shader| shader.0.clone())
    }

    pub fn uniform_values(first: MatrixRow, second: MatrixRow) -> [Uniform<'static>; 4] {
        [
            Uniform::new("first_row", first.row),
            Uniform::new("first_offset", first.offset),
            Uniform::new("second_row", second.row),
            Uniform::new("second_offset", second.offset),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(row: MatrixRow, rgb: [f32; 3]) -> f32 {
        row.row
            .iter()
            .zip(rgb)
            .map(|(weight, channel)| weight * channel)
            .sum::<f32>()
            + row.offset
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 1e-4
    }

    #[test]
    fn black_and_white_land_on_the_limited_range_ends() {
        assert!(close(apply(LUMA, [0.0; 3]), 16.0 / 255.0));
        assert!(close(apply(LUMA, [1.0; 3]), 235.0 / 255.0));
    }

    #[test]
    fn greys_carry_no_chroma() {
        for grey in [0.0, 0.5, 1.0] {
            assert!(close(apply(CHROMA_BLUE, [grey; 3]), 128.0 / 255.0));
            assert!(close(apply(CHROMA_RED, [grey; 3]), 128.0 / 255.0));
        }
    }

    #[test]
    fn primaries_reach_the_chroma_extremes() {
        assert!(close(apply(CHROMA_BLUE, [0.0, 0.0, 1.0]), 240.0 / 255.0));
        assert!(close(apply(CHROMA_RED, [1.0, 0.0, 0.0]), 240.0 / 255.0));
    }

    #[test]
    fn the_shader_declares_every_uniform_it_is_given() {
        for name in ["first_row", "first_offset", "second_row", "second_offset"] {
            assert!(RGB_TO_YCBCR_SHADER.contains(&format!(
                "uniform {}",
                match name {
                    "first_row" | "second_row" => format!("vec3 {name}"),
                    _ => format!("float {name}"),
                }
            )));
        }
        assert!(
            RGB_TO_YCBCR_SHADER
                .lines()
                .any(|line| line == "//_DEFINES_")
        );
    }
}

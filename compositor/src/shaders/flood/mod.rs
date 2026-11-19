use smithay::{
    backend::renderer::gles::{
        GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
    },
    utils::Point,
};

use crate::rendering::{decorate::Flood, framebuffer::FramebufferSpace};

pub static FLOOD_SHADER: &str = include_str!("./flood.frag");

/// Draws the lock's background as far as it has flooded out over the
/// desktop: a blob welling up from a source, its lobes morphing as it
/// spreads, its rim lensing and catching the light.
pub struct FloodShader(pub GlesTexProgram);

impl FloodShader {
    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        let program = renderer.compile_custom_texture_shader(
            FLOOD_SHADER,
            &[
                UniformName::new("origin", UniformType::_2f),
                UniformName::new("across", UniformType::_2f),
                UniformName::new("down", UniformType::_2f),
                UniformName::new("source", UniformType::_2f),
                UniformName::new("aspect", UniformType::_1f),
                UniformName::new("level", UniformType::_1f),
            ],
        )?;
        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| FloodShader(program));
        Ok(())
    }

    pub fn get(renderer: &GlesRenderer) -> Option<GlesTexProgram> {
        renderer
            .egl_context()
            .user_data()
            .get::<FloodShader>()
            .map(|shader| shader.0.clone())
    }

    /// `flood`'s output is output-local, measured into `space`, where the
    /// shader reads `gl_FragCoord`.
    pub fn uniform_values(space: &FramebufferSpace, flood: &Flood) -> [Uniform<'static>; 6] {
        let output = flood.output;
        let corner = |dx: i32, dy: i32| {
            let point = space.point(output.loc + Point::from((dx, dy)));
            [point.x as f32, point.y as f32]
        };
        let origin = corner(0, 0);
        let [right, below] = [corner(output.size.w, 0), corner(0, output.size.h)];
        let across = [right[0] - origin[0], right[1] - origin[1]];
        let down = [below[0] - origin[0], below[1] - origin[1]];
        let determinant = across[0] * down[1] - across[1] * down[0];
        let [to_across, to_down] = if determinant.abs() > f32::EPSILON {
            [
                (down[1] / determinant, -down[0] / determinant),
                (-across[1] / determinant, across[0] / determinant),
            ]
        } else {
            [(0.0, 0.0); 2]
        };
        [
            Uniform::new("origin", (origin[0], origin[1])),
            Uniform::new("across", to_across),
            Uniform::new("down", to_down),
            Uniform::new("source", (flood.source[0], flood.source[1])),
            Uniform::new("aspect", output.size.w as f32 / output.size.h.max(1) as f32),
            Uniform::new("level", flood.level),
        ]
    }
}

#[cfg(test)]
mod tests {
    use smithay::{
        backend::renderer::{element::Id, gles::UniformValue},
        utils::{Rectangle, Size},
    };

    use super::*;

    fn pair(uniform: &Uniform<'_>) -> [f32; 2] {
        match uniform.value {
            UniformValue::_2f(x, y) => [x, y],
            _ => panic!("not a pair"),
        }
    }

    /// The output's bottom-left corner, which GL puts at the framebuffer's
    /// origin, is all the way across 0 and all the way down 1.
    #[test]
    fn the_shader_measures_the_output_from_its_own_top_left() {
        let space = FramebufferSpace::new(
            [2.0 / 100.0, 0.0, 0.0, 0.0, -2.0 / 50.0, 0.0, -1.0, 1.0, 1.0],
            Size::from((100, 50)),
        );
        let flood = Flood {
            id: Id::new(),
            output: Rectangle::from_size((100, 50).into()),
            source: [0.5, 0.5],
            level: 0.5,
        };
        let [origin, across, down, ..] = FloodShader::uniform_values(&space, &flood);
        let [origin, across, down] = [&origin, &across, &down].map(pair);
        assert_eq!(origin, [0.0, 50.0]);
        let offset = [-origin[0], -origin[1]];
        let dot = |row: [f32; 2]| row[0] * offset[0] + row[1] * offset[1];
        assert_eq!([dot(across), dot(down)], [0.0, 1.0]);
    }
}

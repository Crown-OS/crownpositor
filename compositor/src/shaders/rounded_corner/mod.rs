use std::ops::RangeInclusive;

use smithay::{
    backend::renderer::gles::{
        GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Transform},
};

pub static CLIPPING_SHADER: &str = concat!(
    include_str!("./rounded_corner.frag"),
    include_str!("../common/rounded_box.glsl"),
);

/// Buffer pixels per output pixel the area filter handles: its three texels a
/// side cover an output pixel up to two texels wide. Beyond that — overview
/// thumbnails, the wallpaper — plain bilinear sampling is what is left.
const AREA_FILTERED: RangeInclusive<f64> = 0.5..=2.0;

/// How far from 1:1 a draw must be to be resampled. Nearer than this the
/// texels already land on the output pixels.
const ONE_TO_ONE: f64 = 1e-3;

pub struct RoundedCornerShader(pub GlesTexProgram);

impl RoundedCornerShader {
    fn uniforms() -> [UniformName<'static>; 4] {
        [
            UniformName::new("shape_origin", UniformType::_2f),
            UniformName::new("shape_size", UniformType::_2f),
            UniformName::new("radius", UniformType::_4f),
            UniformName::new("resample", UniformType::_2f),
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
    /// framebuffer corner: (+x, +y), (+x, -y), (-x, +y), (-x, -y). `resample`
    /// is what [`Self::resample`] made of the draw.
    pub fn uniform_values(
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
        resample: [f32; 2],
    ) -> [Uniform<'static>; 4] {
        [
            Uniform::new("shape_origin", (shape.loc.x as f32, shape.loc.y as f32)),
            Uniform::new("shape_size", (shape.size.w as f32, shape.size.h as f32)),
            Uniform::new("radius", radius),
            Uniform::new("resample", resample),
        ]
    }

    /// Buffer pixels per output pixel along the buffer's own axes when drawing
    /// `src` into `dst`, for the area filter — or zeros, to sample as is.
    pub fn resample(
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        transform: Transform,
    ) -> [f32; 2] {
        let drawn = transform.transform_size(dst.size);
        let ratio = [
            src.size.w / f64::from(drawn.w),
            src.size.h / f64::from(drawn.h),
        ];
        let filtered = ratio.iter().all(|ratio| AREA_FILTERED.contains(ratio))
            && ratio.iter().any(|ratio| (ratio - 1.0).abs() > ONE_TO_ONE);
        if filtered {
            ratio.map(|ratio| ratio as f32)
        } else {
            [0.0; 2]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resample(src: (f64, f64), dst: (i32, i32), transform: Transform) -> [f32; 2] {
        RoundedCornerShader::resample(
            Rectangle::from_size(src.into()),
            Rectangle::from_size(dst.into()),
            transform,
        )
    }

    #[test]
    fn a_buffer_drawn_one_to_one_is_sampled_as_is() {
        assert_eq!(
            resample((946.0, 600.0), (946, 600), Transform::Normal),
            [0.0; 2]
        );
    }

    #[test]
    fn a_2x_buffer_on_a_1_25_output_is_area_filtered() {
        assert_eq!(
            resample((1600.0, 960.0), (1000, 600), Transform::Normal),
            [1.6; 2]
        );
    }

    #[test]
    fn a_1x_buffer_on_a_1_25_output_is_area_filtered() {
        assert_eq!(
            resample((800.0, 480.0), (1000, 600), Transform::Normal),
            [0.8; 2]
        );
    }

    #[test]
    fn a_thumbnail_falls_back_to_bilinear() {
        assert_eq!(
            resample((1600.0, 960.0), (400, 240), Transform::Normal),
            [0.0; 2]
        );
    }

    #[test]
    fn a_rotated_buffer_is_measured_along_its_own_axes() {
        assert_eq!(
            resample((960.0, 1600.0), (1000, 600), Transform::_90),
            [1.6; 2]
        );
    }

    #[test]
    fn an_empty_draw_is_not_divided_by() {
        assert_eq!(
            resample((946.0, 600.0), (0, 0), Transform::Normal),
            [0.0; 2]
        );
    }
}

//! Output-local physical coordinates mapped into framebuffer pixels.
//!
//! Shaders that measure a shape from `gl_FragCoord` — the blur's glass, a
//! window's rounded clip — need it in the framebuffer's own space. The frame's
//! projection already carries the output transform, so this is the single
//! place a rotated or flipped output is dealt with: every rotation and flip
//! keeps rectangles axis-aligned, which is all a shape needs.

use smithay::{
    backend::renderer::gles::{GlesError, GlesFrame, ffi},
    utils::{Physical, Point, Rectangle, Size},
};

pub struct FramebufferSpace {
    projection: [f32; 9],
    viewport: Size<i32, Physical>,
}

impl FramebufferSpace {
    pub fn new(projection: [f32; 9], viewport: Size<i32, Physical>) -> Self {
        Self {
            projection,
            viewport,
        }
    }

    /// The space `frame` is drawing in right now.
    pub fn current(frame: &mut GlesFrame<'_, '_>) -> Result<Self, GlesError> {
        let projection = *frame.projection();
        let mut viewport = [0; 4];
        frame.with_context(|gl| unsafe { gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr()) })?;
        Ok(Self::new(
            projection,
            Size::from((viewport[2], viewport[3])),
        ))
    }

    pub fn viewport(&self) -> Size<i32, Physical> {
        self.viewport
    }

    pub fn point(&self, point: Point<i32, Physical>) -> Point<i32, Physical> {
        let matrix = &self.projection;
        let (x, y) = (point.x as f32, point.y as f32);
        let ndc = (
            matrix[0] * x + matrix[3] * y + matrix[6],
            matrix[1] * x + matrix[4] * y + matrix[7],
        );
        Point::from((
            ((ndc.0 + 1.0) * 0.5 * self.viewport.w as f32).round() as i32,
            ((ndc.1 + 1.0) * 0.5 * self.viewport.h as f32).round() as i32,
        ))
    }

    /// A direction in output-local coordinates, as a unit vector in
    /// framebuffer pixels. Only the projection's linear part is involved: a
    /// direction has no origin to translate.
    pub fn direction(&self, delta: (f32, f32)) -> (f32, f32) {
        let matrix = &self.projection;
        let x = (matrix[0] * delta.0 + matrix[3] * delta.1) * self.viewport.w as f32;
        let y = (matrix[1] * delta.0 + matrix[4] * delta.1) * self.viewport.h as f32;
        let length = x.hypot(y);
        if length > 0.0 {
            (x / length, y / length)
        } else {
            (0.0, 0.0)
        }
    }

    pub fn rect(&self, rect: Rectangle<i32, Physical>) -> Rectangle<i32, Physical> {
        let start = self.point(rect.loc);
        let end = self.point(rect.loc + rect.size.to_point());
        Rectangle::from_extremities(
            (start.x.min(end.x), start.y.min(end.y)),
            (start.x.max(end.x), start.y.max(end.y)),
        )
    }

    /// Per-corner radii given for the output's (+x, +y), (+x, -y), (-x, +y)
    /// and (-x, -y) corners, reordered to the framebuffer's corners in that
    /// same order: a flip or a rotation moves which corner is which.
    pub fn corners(&self, radius: [f32; 4]) -> [f32; 4] {
        const SIGNS: [(f32, f32); 4] = [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)];
        let mut mapped = [0.0; 4];
        for (&(x, y), &r) in SIGNS.iter().zip(&radius) {
            let (fx, fy) = self.direction((x, y));
            mapped[corner_slot(fx, fy)] = r;
        }
        mapped
    }
}

fn corner_slot(x: f32, y: f32) -> usize {
    match (x > 0.0, y > 0.0) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What smithay builds for an untransformed output: x right, y flipped so
    /// that output-down is framebuffer-down in GL's bottom-up space.
    fn y_flipped() -> FramebufferSpace {
        FramebufferSpace::new(
            [2.0 / 100.0, 0.0, 0.0, 0.0, -2.0 / 50.0, 0.0, -1.0, 1.0, 1.0],
            Size::from((100, 50)),
        )
    }

    #[test]
    fn a_rect_keeps_its_size_and_lands_flipped() {
        let space = y_flipped();
        let rect = space.rect(Rectangle::new((10, 5).into(), (20, 10).into()));
        assert_eq!(rect, Rectangle::new((10, 35).into(), (20, 10).into()));
    }

    #[test]
    fn a_vertical_flip_swaps_top_and_bottom_corners() {
        let space = y_flipped();
        // Output bottom-right becomes the framebuffer's (+x, -y) corner.
        assert_eq!(space.corners([1.0, 2.0, 3.0, 4.0]), [2.0, 1.0, 4.0, 3.0]);
    }
}

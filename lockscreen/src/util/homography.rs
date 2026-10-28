/// A projective map of the plane, as a row-major 3×3 matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography([f32; 9]);

/// Below this a determinant or `w` counts as zero.
const DEGENERATE: f32 = 1e-9;

impl Homography {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    /// Scales by `scale`, then moves by `shift`.
    pub fn affine(scale: [f32; 2], shift: [f32; 2]) -> Self {
        Self([
            scale[0], 0.0, shift[0], 0.0, scale[1], shift[1], 0.0, 0.0, 1.0,
        ])
    }

    /// Maps the box at `origin` sized `size` onto `quad`: top left, top
    /// right, bottom right, bottom left. `None` when the quad folds over.
    pub fn box_to_quad(origin: [f32; 2], size: [f32; 2], quad: [[f32; 2]; 4]) -> Option<Self> {
        if size[0] <= 0.0 || size[1] <= 0.0 {
            return None;
        }
        let into_unit = Self::affine(
            [1.0 / size[0], 1.0 / size[1]],
            [-origin[0] / size[0], -origin[1] / size[1]],
        );
        Some(Self::unit_to_quad(quad)?.after(into_unit))
    }

    /// Heckbert's square-to-quad map.
    fn unit_to_quad([[x0, y0], [x1, y1], [x2, y2], [x3, y3]]: [[f32; 2]; 4]) -> Option<Self> {
        let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
        let (dx1, dy1, dx2, dy2) = (x1 - x2, y1 - y2, x3 - x2, y3 - y2);
        let det = dx1 * dy2 - dx2 * dy1;
        if det.abs() < DEGENERATE {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / det;
        let h = (dx1 * sy - sx * dy1) / det;
        Some(Self([
            x1 - x0 + g * x1,
            x3 - x0 + h * x3,
            x0,
            y1 - y0 + g * y1,
            y3 - y0 + h * y3,
            y0,
            g,
            h,
            1.0,
        ]))
    }

    /// `first`, then `self`.
    pub fn after(self, first: Self) -> Self {
        let (a, b) = (self.0, first.0);
        Self(std::array::from_fn(|index| {
            let (row, column) = (index / 3, index % 3);
            (0..3).map(|k| a[row * 3 + k] * b[k * 3 + column]).sum()
        }))
    }

    pub fn inverse(self) -> Option<Self> {
        let [a, b, c, d, e, f, g, h, i] = self.0;
        let cofactors = [
            e * i - f * h,
            c * h - b * i,
            b * f - c * e,
            f * g - d * i,
            a * i - c * g,
            c * d - a * f,
            d * h - e * g,
            b * g - a * h,
            a * e - b * d,
        ];
        let det = a * cofactors[0] + b * cofactors[3] + c * cofactors[6];
        (det.abs() >= DEGENERATE).then(|| Self(cofactors.map(|value| value / det)))
    }

    /// Where `point` lands, or `None` when it maps to infinity.
    pub fn apply(&self, [x, y]: [f32; 2]) -> Option<[f32; 2]> {
        let m = &self.0;
        let w = m[6] * x + m[7] * y + m[8];
        (w.abs() >= DEGENERATE).then(|| {
            [
                (m[0] * x + m[1] * y + m[2]) / w,
                (m[3] * x + m[4] * y + m[5]) / w,
            ]
        })
    }

    /// Column-major, as GLSL's `mat3` takes it.
    pub fn columns(&self) -> [f32; 9] {
        let m = &self.0;
        [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    const QUAD: [[f32; 2]; 4] = [[10.0, 20.0], [130.0, 5.0], [140.0, 90.0], [0.0, 70.0]];

    #[test]
    fn the_box_corners_land_on_the_quad() {
        let map = Homography::box_to_quad([100.0, 50.0], [40.0, 20.0], QUAD).expect("convex");
        let corners = [[100.0, 50.0], [140.0, 50.0], [140.0, 70.0], [100.0, 70.0]];
        for (corner, target) in corners.into_iter().zip(QUAD) {
            assert!(
                close(map.apply(corner).expect("finite"), target),
                "{corner:?}"
            );
        }
    }

    #[test]
    fn the_inverse_undoes_the_map() {
        let map = Homography::box_to_quad([0.0, 0.0], [1.0, 1.0], QUAD).expect("convex");
        let back = map.inverse().expect("invertible");
        let point = [0.3, 0.8];
        let there = map.apply(point).expect("finite");
        assert!(close(back.apply(there).expect("finite"), point));
    }

    #[test]
    fn an_unwarped_box_maps_to_itself() {
        let quad = [[2.0, 3.0], [12.0, 3.0], [12.0, 8.0], [2.0, 8.0]];
        let map = Homography::box_to_quad([2.0, 3.0], [10.0, 5.0], quad).expect("box");
        assert!(close(map.apply([7.0, 4.0]).expect("finite"), [7.0, 4.0]));
    }

    #[test]
    fn a_collapsed_quad_has_no_map() {
        let flat = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
        assert_eq!(Homography::box_to_quad([0.0; 2], [1.0; 2], flat), None);
    }
}

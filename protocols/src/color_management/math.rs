//! Colorimetry: chromaticities in, matrices out.
//!
//! Everything here is pure arithmetic over CIE 1931 xy coordinates, which is
//! what makes it the only part of colour management that can be tested
//! without a display. It is also the part that is easiest to get subtly wrong
//! — a transposed matrix looks *almost* right — so the tests compare against
//! the published matrices rather than against this code's own output.

/// A CIE 1931 xy chromaticity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chromaticity {
    pub x: f64,
    pub y: f64,
}

impl Chromaticity {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The XYZ of this chromaticity at unit luminance.
    fn to_xyz(self) -> [f64; 3] {
        if self.y == 0.0 {
            return [0.0, 0.0, 0.0];
        }
        [self.x / self.y, 1.0, (1.0 - self.x - self.y) / self.y]
    }
}

/// The three primaries and the white point of a colour volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Primaries {
    pub red: Chromaticity,
    pub green: Chromaticity,
    pub blue: Chromaticity,
    pub white: Chromaticity,
}

/// A row-major 3x3 matrix.
pub type Matrix3 = [[f64; 3]; 3];

/// The named primaries the protocol defines, with their published values.
pub mod named {
    use super::{Chromaticity as C, Primaries};

    const D65: C = C::new(0.3127, 0.3290);
    const D50: C = C::new(0.34567, 0.35850);
    /// DCI's white, which is green-ward of daylight.
    const DCI: C = C::new(0.314, 0.351);

    pub const SRGB: Primaries = Primaries {
        red: C::new(0.640, 0.330),
        green: C::new(0.300, 0.600),
        blue: C::new(0.150, 0.060),
        white: D65,
    };

    pub const PAL_M: Primaries = Primaries {
        red: C::new(0.67, 0.33),
        green: C::new(0.21, 0.71),
        blue: C::new(0.14, 0.08),
        white: C::new(0.310, 0.316),
    };

    pub const PAL: Primaries = Primaries {
        red: C::new(0.640, 0.330),
        green: C::new(0.290, 0.600),
        blue: C::new(0.150, 0.060),
        white: D65,
    };

    pub const NTSC: Primaries = Primaries {
        red: C::new(0.630, 0.340),
        green: C::new(0.310, 0.595),
        blue: C::new(0.155, 0.070),
        white: D65,
    };

    pub const GENERIC_FILM: Primaries = Primaries {
        red: C::new(0.681, 0.319),
        green: C::new(0.243, 0.692),
        blue: C::new(0.145, 0.049),
        white: C::new(0.310, 0.316),
    };

    pub const BT2020: Primaries = Primaries {
        red: C::new(0.708, 0.292),
        green: C::new(0.170, 0.797),
        blue: C::new(0.131, 0.046),
        white: D65,
    };

    /// The identity volume: the primaries *are* the XYZ axes.
    pub const CIE1931_XYZ: Primaries = Primaries {
        red: C::new(1.0, 0.0),
        green: C::new(0.0, 1.0),
        blue: C::new(0.0, 0.0),
        white: D50,
    };

    pub const DCI_P3: Primaries = Primaries {
        red: C::new(0.680, 0.320),
        green: C::new(0.265, 0.690),
        blue: C::new(0.150, 0.060),
        white: DCI,
    };

    pub const DISPLAY_P3: Primaries = Primaries {
        red: C::new(0.680, 0.320),
        green: C::new(0.265, 0.690),
        blue: C::new(0.150, 0.060),
        white: D65,
    };

    pub const ADOBE_RGB: Primaries = Primaries {
        red: C::new(0.640, 0.330),
        green: C::new(0.210, 0.710),
        blue: C::new(0.150, 0.060),
        white: D65,
    };
}

/// The matrix taking linear RGB in this volume to CIE XYZ.
///
/// The standard construction: scale each primary's unit-luminance XYZ so that
/// `(1, 1, 1)` lands exactly on the white point.
pub fn rgb_to_xyz(primaries: Primaries) -> Option<Matrix3> {
    let red = primaries.red.to_xyz();
    let green = primaries.green.to_xyz();
    let blue = primaries.blue.to_xyz();

    let basis = [
        [red[0], green[0], blue[0]],
        [red[1], green[1], blue[1]],
        [red[2], green[2], blue[2]],
    ];
    let scale = multiply_vector(invert(basis)?, primaries.white.to_xyz());

    Some([
        [red[0] * scale[0], green[0] * scale[1], blue[0] * scale[2]],
        [red[1] * scale[0], green[1] * scale[1], blue[1] * scale[2]],
        [red[2] * scale[0], green[2] * scale[1], blue[2] * scale[2]],
    ])
}

/// The Bradford cone response matrix, for chromatic adaptation.
const BRADFORD: Matrix3 = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

/// The matrix adapting XYZ measured under `from` to appear under `to`.
///
/// Without this a white point difference reads as a colour cast: content
/// mastered at DCI white shown on a D65 display would come out green.
pub fn chromatic_adaptation(from: Chromaticity, to: Chromaticity) -> Option<Matrix3> {
    let source = multiply_vector(BRADFORD, from.to_xyz());
    let target = multiply_vector(BRADFORD, to.to_xyz());

    let ratio = [
        [safe_ratio(target[0], source[0]), 0.0, 0.0],
        [0.0, safe_ratio(target[1], source[1]), 0.0],
        [0.0, 0.0, safe_ratio(target[2], source[2])],
    ];

    Some(multiply(
        multiply(invert(BRADFORD)?, ratio),
        BRADFORD,
    ))
}

fn safe_ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 { 1.0 } else { numerator / denominator }
}

/// The full conversion from one volume's linear RGB to another's.
///
/// `adapt` is what the `absolute_no_adaptation` render intent turns off: it is
/// the only place the intent changes the matrix.
pub fn convert(from: Primaries, to: Primaries, adapt: bool) -> Option<Matrix3> {
    let source = rgb_to_xyz(from)?;
    let destination = invert(rgb_to_xyz(to)?)?;

    let matrix = if adapt && from.white != to.white {
        multiply(chromatic_adaptation(from.white, to.white)?, source)
    } else {
        source
    };

    Some(multiply(destination, matrix))
}

pub fn multiply(left: Matrix3, right: Matrix3) -> Matrix3 {
    let mut out = [[0.0; 3]; 3];
    for (row, values) in out.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = (0..3).map(|k| left[row][k] * right[k][column]).sum();
        }
    }
    out
}

pub fn multiply_vector(matrix: Matrix3, vector: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|row| (0..3).map(|k| matrix[row][k] * vector[k]).sum())
}

/// `None` for a singular matrix, which is what a degenerate set of primaries
/// produces — three collinear points describe no volume at all.
pub fn invert(matrix: Matrix3) -> Option<Matrix3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = matrix;

    let cofactors = [
        [e * i - f * h, c * h - b * i, b * f - c * e],
        [f * g - d * i, a * i - c * g, c * d - a * f],
        [d * h - e * g, b * g - a * h, a * e - b * d],
    ];
    let determinant = a * cofactors[0][0] + b * cofactors[1][0] + c * cofactors[2][0];
    if determinant.abs() < 1e-12 {
        return None;
    }

    Some(cofactors.map(|row| row.map(|value| value / determinant)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: Matrix3, expected: Matrix3, tolerance: f64, what: &str) {
        for row in 0..3 {
            for column in 0..3 {
                let difference = (actual[row][column] - expected[row][column]).abs();
                assert!(
                    difference < tolerance,
                    "{what}: [{row}][{column}] was {} not {} (off by {difference})",
                    actual[row][column],
                    expected[row][column]
                );
            }
        }
    }

    #[test]
    fn srgb_to_xyz_matches_the_published_matrix() {
        // IEC 61966-2-1, D65.
        let expected = [
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ];
        let actual = rgb_to_xyz(named::SRGB).expect("sRGB is not degenerate");
        assert_close(actual, expected, 1e-4, "sRGB->XYZ");
    }

    #[test]
    fn bt2020_to_xyz_matches_the_published_matrix() {
        let expected = [
            [0.6370, 0.1446, 0.1689],
            [0.2627, 0.6780, 0.0593],
            [0.0000, 0.0281, 1.0610],
        ];
        let actual = rgb_to_xyz(named::BT2020).expect("BT.2020 is not degenerate");
        assert_close(actual, expected, 1e-3, "BT.2020->XYZ");
    }

    #[test]
    fn white_maps_to_the_white_point() {
        for (name, primaries) in [
            ("sRGB", named::SRGB),
            ("BT.2020", named::BT2020),
            ("Display P3", named::DISPLAY_P3),
            ("Adobe RGB", named::ADOBE_RGB),
        ] {
            let matrix = rgb_to_xyz(primaries).expect("a real volume");
            let white = multiply_vector(matrix, [1.0, 1.0, 1.0]);
            let expected = primaries.white.to_xyz();
            assert!(
                (white[0] - expected[0]).abs() < 1e-9 && (white[2] - expected[2]).abs() < 1e-9,
                "{name}: (1,1,1) must land on the white point"
            );
        }
    }

    #[test]
    fn converting_a_volume_to_itself_is_the_identity() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let actual = convert(named::SRGB, named::SRGB, true).expect("identity conversion");
        assert_close(actual, identity, 1e-9, "sRGB->sRGB");
    }

    #[test]
    fn a_round_trip_through_another_volume_returns_the_original() {
        let there = convert(named::SRGB, named::BT2020, true).expect("there");
        let back = convert(named::BT2020, named::SRGB, true).expect("back");
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert_close(multiply(back, there), identity, 1e-9, "sRGB->BT.2020->sRGB");
    }

    #[test]
    fn srgb_inside_bt2020_stays_inside_it() {
        // Every sRGB primary is representable in BT.2020, so no channel may go
        // negative — that is what "wider gamut" means.
        let matrix = convert(named::SRGB, named::BT2020, true).expect("a conversion");
        for corner in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            for channel in multiply_vector(matrix, corner) {
                assert!(channel > -1e-6, "sRGB must fit inside BT.2020");
            }
        }
    }

    #[test]
    fn adaptation_moves_one_white_point_onto_the_other() {
        let matrix = chromatic_adaptation(named::DCI_P3.white, named::SRGB.white)
            .expect("an adaptation");
        let adapted = multiply_vector(matrix, named::DCI_P3.white.to_xyz());
        let expected = named::SRGB.white.to_xyz();

        for axis in 0..3 {
            assert!(
                (adapted[axis] - expected[axis]).abs() < 1e-6,
                "DCI white must adapt onto D65"
            );
        }
    }

    #[test]
    fn skipping_adaptation_leaves_the_white_points_apart() {
        let adapted = convert(named::DCI_P3, named::SRGB, true).expect("adapted");
        let raw = convert(named::DCI_P3, named::SRGB, false).expect("not adapted");
        assert!(
            (adapted[0][0] - raw[0][0]).abs() > 1e-3,
            "adaptation must actually change the matrix"
        );
    }

    #[test]
    fn degenerate_primaries_have_no_matrix() {
        let collinear = Primaries {
            red: Chromaticity::new(0.3, 0.3),
            green: Chromaticity::new(0.4, 0.4),
            blue: Chromaticity::new(0.5, 0.5),
            white: Chromaticity::new(0.3127, 0.3290),
        };
        assert_eq!(rgb_to_xyz(collinear), None);
    }

    #[test]
    fn a_singular_matrix_cannot_be_inverted() {
        assert_eq!(invert([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [1.0, 1.0, 1.0]]), None);
    }
}

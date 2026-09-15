//! The one colour encoding this protocol uses.

/// An `ARGB8888` colour as the protocol carries it: straight (not
/// premultiplied) alpha, in the surface's own colour space.
///
/// Kept packed rather than unpacked into four floats because it is compared far
/// more often than it is read — every commit checks whether the effects
/// changed, and one `u32` comparison is the whole of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Argb(pub u32);

impl Argb {
    pub const TRANSPARENT: Self = Self(0);

    /// Straight RGBA in `0.0..=1.0`, the order a shader uniform wants.
    pub fn channels(self) -> [f32; 4] {
        let byte = |shift: u32| ((self.0 >> shift) & 0xff) as f32 / 255.0;
        [byte(16), byte(8), byte(0), byte(24)]
    }

    /// The same colour with every channel scaled by its alpha, which is what a
    /// premultiplied render target expects.
    pub fn premultiplied(self) -> [f32; 4] {
        let [red, green, blue, alpha] = self.channels();
        [red * alpha, green * alpha, blue * alpha, alpha]
    }

    pub fn is_transparent(self) -> bool {
        self.0 >> 24 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_unpack_in_rgba_order() {
        // 0xAARRGGBB: opaque pure red.
        assert_eq!(Argb(0xff_ff_00_00).channels(), [1.0, 0.0, 0.0, 1.0]);
        // Half-transparent pure blue.
        let [red, green, blue, alpha] = Argb(0x80_00_00_ff).channels();
        assert_eq!((red, green, blue), (0.0, 0.0, 1.0));
        assert!((alpha - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn premultiplication_scales_colour_not_alpha() {
        let [red, green, blue, alpha] = Argb(0x80_ff_00_00).premultiplied();
        assert_eq!((green, blue), (0.0, 0.0));
        assert_eq!(red, alpha);
    }

    #[test]
    fn a_zero_alpha_colour_is_transparent_whatever_its_channels() {
        assert!(Argb(0x00_ff_ff_ff).is_transparent());
        assert!(!Argb(0x01_00_00_00).is_transparent());
    }
}

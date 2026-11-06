//! Glass digits as ColorOS 17 draws them, fitted to its screenshots: the
//! background behind the digit, blurred and brightened, with white added on
//! top of it — a plus-lighter blend on sRGB-encoded values, clamped — a
//! little more white in a band along the glyph's top, and an inner rim.

use crownos_config::schema::ClockLook;
use tiny_skia::Pixmap;

/// Of the glyph's height, top to baseline: the Gaussian sigma the background
/// is blurred by behind the digit.
pub const FROST_SIGMA: f32 = 0.045;
/// Of the glyph's height: how far down the top shine fades out.
pub const TOP_SHINE_REACH: f32 = 0.12;
/// Of the glyph's height: how deep the inner rim reaches.
pub const RIM_REACH: f32 = 0.006;
/// White added along the inner rim.
pub const RIM_WHITE: f32 = 0.12;
/// How much of the lock's darkening the frost behind a digit carries: none.
/// The glass shows the background as it is, so the digits read the same
/// over any dim, as their whiteness was tuned.
pub const FROST_DARKENING: f32 = 0.0;

/// Pixels: the sigma the background is blurred by behind a glyph `height`
/// pixels tall.
pub fn frost_sigma(height: f32) -> f32 {
    FROST_SIGMA * height
}

/// Pixels the inner rim reaches into a glyph `height` pixels tall.
pub fn rim_reach(height: f32) -> f32 {
    RIM_REACH * height
}

/// How much of the inner rim a pixel of `coverage` is, `eroded` being its
/// coverage once the glyph is eroded by [`rim_reach`]: what erosion takes.
pub fn rim(coverage: f32, eroded: f32) -> f32 {
    (coverage - eroded).clamp(0.0, 1.0)
}

/// [`rim`] at `pixel` of `coverage`, a glyph `height` pixels tall in white,
/// as the glass shader works it out: eroded to the least coverage
/// [`rim_reach`] to either side, above and below, read between pixels.
pub fn rim_at(coverage: &Pixmap, [column, row]: [usize; 2], height: f32) -> f32 {
    let reach = rim_reach(height);
    let [x, y] = [column as f32, row as f32];
    let eroded = [[reach, 0.0], [-reach, 0.0], [0.0, reach], [0.0, -reach]]
        .into_iter()
        .map(|[across, down]| alpha_between(coverage, x + across, y + down))
        .fold(f32::MAX, f32::min);
    rim(alpha_between(coverage, x, y), eroded)
}

/// The alpha of `pixmap`, 0 to 1, at `x`, `y` in pixels, read bilinearly
/// between pixel centres; transparent past its edges.
fn alpha_between(pixmap: &Pixmap, x: f32, y: f32) -> f32 {
    let (left, top) = (x.floor(), y.floor());
    let (across, down) = (x - left, y - top);
    let alpha = |column: f32, row: f32| {
        let inside = column >= 0.0 && row >= 0.0;
        inside
            .then(|| pixmap.pixel(column as u32, row as u32))
            .flatten()
            .map_or(0.0, |pixel| f32::from(pixel.alpha()) / 255.0)
    };
    let upper = alpha(left, top) + (alpha(left + 1.0, top) - alpha(left, top)) * across;
    let lower =
        alpha(left, top + 1.0) + (alpha(left + 1.0, top + 1.0) - alpha(left, top + 1.0)) * across;
    upper + (lower - upper) * down
}

/// The glass at one whiteness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glass {
    /// What the blurred background is multiplied by.
    pub gain: f32,
    /// White added all over the digit.
    pub white: f32,
    /// White added on top of `white` at the glyph's top, gone by
    /// [`TOP_SHINE_REACH`].
    pub top_shine: f32,
}

const CLEAR: Glass = Glass {
    gain: 1.0,
    white: 0.03,
    top_shine: 0.01,
};
const STOCK: Glass = Glass {
    gain: 1.25,
    white: 0.235,
    top_shine: 0.05,
};
const SOLID: Glass = Glass {
    gain: 1.25,
    white: 1.0,
    top_shine: 0.0,
};

impl Glass {
    /// The glass for a clock `whiteness` from clear frosted glass, 0, through
    /// ColorOS's own at [`ClockLook::DEFAULT_WHITENESS`], to solid white, 1.
    pub fn at(whiteness: f32) -> Self {
        let whiteness = whiteness.clamp(0.0, 1.0);
        let stock = ClockLook::DEFAULT_WHITENESS;
        if whiteness < stock {
            CLEAR.toward(STOCK, whiteness / stock)
        } else {
            STOCK.toward(SOLID, (whiteness - stock) / (1.0 - stock))
        }
    }

    /// The white added `down` the glyph (0 at its top, 1 at its baseline),
    /// `rim` (0..=1) of the way into the inner rim.
    pub fn white_at(&self, down: f32, rim: f32) -> f32 {
        let shine = 1.0 - smoothstep(0.0, TOP_SHINE_REACH, down);
        self.white + self.top_shine * shine + RIM_WHITE * rim.clamp(0.0, 1.0)
    }

    /// One sRGB-encoded channel of the digit's face over `frost`, the
    /// background blurred by [`FROST_SIGMA`].
    pub fn face(&self, frost: f32, down: f32, rim: f32) -> f32 {
        (frost * self.gain + self.white_at(down, rim)).min(1.0)
    }

    fn toward(self, other: Self, share: f32) -> Self {
        let lerp = |from: f32, to: f32| from + (to - from) * share;
        Self {
            gain: lerp(self.gain, other.gain),
            white: lerp(self.white, other.white),
            top_shine: lerp(self.top_shine, other.top_shine),
        }
    }
}

fn smoothstep(from: f32, to: f32, value: f32) -> f32 {
    let t = ((value - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whiteness_runs_from_clear_glass_through_coloros_to_solid_white() {
        assert_eq!(Glass::at(0.0), CLEAR);
        assert_eq!(Glass::at(ClockLook::DEFAULT_WHITENESS), STOCK);
        assert_eq!(Glass::at(1.0), SOLID);
        assert_eq!(Glass::at(7.0), SOLID);
        let between = Glass::at(0.75);
        assert!(between.white > STOCK.white && between.white < SOLID.white);
    }

    #[test]
    fn the_face_brightens_the_frost_and_clamps() {
        let stock = Glass::at(ClockLook::DEFAULT_WHITENESS);
        let deep_sky = 0.3;
        assert!((stock.face(deep_sky, 0.5, 0.0) - (0.3 * 1.25 + 0.235)).abs() < 1e-6);
        assert_eq!(stock.face(0.9, 0.5, 0.0), 1.0);
        assert_eq!(Glass::at(1.0).face(0.0, 0.5, 0.0), 1.0);
    }

    #[test]
    fn the_rim_is_what_erosion_takes_from_the_edge() {
        let mut square = Pixmap::new(40, 40).expect("size");
        for row in square.data_mut().chunks_exact_mut(40 * 4).skip(10).take(20) {
            row[10 * 4..30 * 4].fill(u8::MAX);
        }
        let one_pixel = 1.0 / RIM_REACH;
        assert_eq!(rim_at(&square, [10, 20], one_pixel), 1.0, "the edge");
        assert_eq!(rim_at(&square, [12, 20], one_pixel), 0.0, "inside");
        assert_eq!(rim_at(&square, [5, 20], one_pixel), 0.0, "outside");
        assert!((rim_at(&square, [10, 20], one_pixel / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(rim(0.4, 0.9), 0.0);
    }

    #[test]
    fn the_top_shines_and_the_rim_glows() {
        let stock = Glass::at(ClockLook::DEFAULT_WHITENESS);
        assert!(stock.white_at(0.0, 0.0) > stock.white_at(0.5, 0.0));
        assert_eq!(stock.white_at(TOP_SHINE_REACH, 0.0), stock.white);
        assert!((stock.white_at(0.5, 1.0) - stock.white - RIM_WHITE).abs() < 1e-6);
    }
}

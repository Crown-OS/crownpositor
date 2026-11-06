//! The background's subject, cut out of it by a mask, to stand in front of
//! the clock.

use tiny_skia::{IntRect, Pixmap};

use super::{ImageError, limited_reader};
use crate::layout::Rect;

/// A grayscale mask: one byte a pixel, row by row, 255 where the
/// background's subject is and 0 where it is not. Of any size with the
/// background's source aspect: [`subject`] covers it to the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    width: u32,
    height: u32,
    coverage: Vec<u8>,
}

impl Mask {
    /// `None` unless `coverage` holds exactly `width` × `height` bytes, and
    /// at least one.
    pub fn new(width: u32, height: u32, coverage: Vec<u8>) -> Option<Self> {
        let pixels = usize::try_from(u64::from(width) * u64::from(height)).ok()?;
        (pixels > 0 && coverage.len() == pixels).then_some(Self {
            width,
            height,
            coverage,
        })
    }

    /// Decodes a grayscale picture, like the PNG `crown-depthcut` writes,
    /// under the limits of any lock-screen picture; colour reads as its
    /// luminance.
    pub fn decode(bytes: &[u8]) -> Result<Self, ImageError> {
        let luma = limited_reader(bytes)?.decode()?.into_luma8();
        let (width, height) = luma.dimensions();
        Self::new(width, height, luma.into_raw()).ok_or(ImageError::Empty)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The coverage, `0..=255`, at `x`, `y` in mask pixels measured between
    /// pixel centres: bilinear, and held at the edges.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let (last_column, last_row) = (self.width as usize - 1, self.height as usize - 1);
        let (x, y) = (
            x.clamp(0.0, last_column as f32),
            y.clamp(0.0, last_row as f32),
        );
        let (left, top) = (x.floor() as usize, y.floor() as usize);
        let (right, bottom) = ((left + 1).min(last_column), (top + 1).min(last_row));
        let (across, down) = (x.fract(), y.fract());
        let at = |column: usize, row: usize| {
            f32::from(self.coverage[row * self.width as usize + column])
        };
        let upper = at(left, top) + (at(right, top) - at(left, top)) * across;
        let lower = at(left, bottom) + (at(right, bottom) - at(left, bottom)) * across;
        upper + (lower - upper) * down
    }
}

/// `background` where `mask` covers it and transparent elsewhere: the
/// picture drawn in front of the clock for depth. The mask is scaled to
/// cover the background and centred, as [`cover`](super::cover) frames a
/// picture to an output, so a mask cut from the picture a covered
/// background came from lines up with it.
pub fn subject(background: &Pixmap, mask: &Mask) -> Pixmap {
    let (width, height) = (background.width() as f32, background.height() as f32);
    let (mask_width, mask_height) = (mask.width as f32, mask.height as f32);
    let scale = (width / mask_width).max(height / mask_height);
    let origin = [
        (width - mask_width * scale) / 2.0,
        (height - mask_height * scale) / 2.0,
    ];
    let into_mask = |at: f32, origin: f32| (at + 0.5 - origin) / scale - 0.5;
    let columns: Vec<f32> = (0..background.width())
        .map(|column| into_mask(column as f32, origin[0]))
        .collect();
    let mut subject = background.clone();
    let row_bytes = columns.len() * 4;
    for (row, pixels) in subject.data_mut().chunks_exact_mut(row_bytes).enumerate() {
        let y = into_mask(row as f32, origin[1]);
        for (pixel, &x) in pixels.chunks_exact_mut(4).zip(&columns) {
            let kept = mask.sample(x, y) / 255.0;
            for channel in pixel {
                *channel = (f32::from(*channel) * kept).round() as u8;
            }
        }
    }
    subject
}

/// The most of the clock a subject may hide before the depth effect gives
/// way and the clock is drawn in front, as ColorOS keeps the time legible.
pub const MAX_DEPTH_OCCLUSION: f32 = 0.35;

/// How much of `area`, in the subject's pixels, the subject covers: 0 for
/// none of it, 1 for all of it. The part of `area` off the subject counts as
/// uncovered.
pub fn occlusion(subject: &Pixmap, area: IntRect) -> f32 {
    let Some(inside) = IntRect::from_xywh(0, 0, subject.width(), subject.height())
        .and_then(|bounds| bounds.intersect(&area))
    else {
        return 0.0;
    };
    let row_bytes = subject.width() as usize * 4;
    let columns = inside.left() as usize * 4..inside.right() as usize * 4;
    let covered: u64 = subject
        .data()
        .chunks_exact(row_bytes)
        .skip(inside.top() as usize)
        .take(inside.height() as usize)
        .flat_map(|row| row[columns.clone()].iter().skip(3).step_by(4))
        .map(|alpha| u64::from(*alpha))
        .sum();
    covered as f32 / (255.0 * area.width() as f32 * area.height() as f32)
}

/// How much of a clock in `clock`, logical pixels on an `output` of that
/// logical size, a `subject` covering the output covers once drawn `zoom`
/// times larger about the output's centre: 0 for none of it, 1 for all.
pub fn clock_occlusion(subject: &Pixmap, clock: Rect, output: [f32; 2], zoom: f32) -> f32 {
    let size = [subject.width() as f32, subject.height() as f32];
    let into_subject = |at: f32, axis: usize| {
        let centre = size[axis] / 2.0;
        centre + (at * size[axis] / output[axis].max(1.0) - centre) / zoom.max(f32::EPSILON)
    };
    IntRect::from_ltrb(
        into_subject(clock.x, 0).floor() as i32,
        into_subject(clock.y, 1).floor() as i32,
        into_subject(clock.x + clock.width, 0).ceil() as i32,
        into_subject(clock.y + clock.height, 1).ceil() as i32,
    )
    .map_or(0.0, |area| occlusion(subject, area))
}

/// Whether a `subject` in front of the clock would hide more of it than
/// [`MAX_DEPTH_OCCLUSION`], measured as [`clock_occlusion`] does: the depth
/// effect then gives way and the clock is drawn in front.
pub fn hides_clock(subject: &Pixmap, clock: Rect, output: [f32; 2], zoom: f32) -> bool {
    clock_occlusion(subject, clock, output, zoom) > MAX_DEPTH_OCCLUSION
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tiny_skia::Color;

    use super::*;

    fn opaque(width: u32, height: u32) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height).expect("size");
        pixmap.fill(Color::from_rgba8(200, 100, 50, 255));
        pixmap
    }

    /// The left half of a `width` × `height` mask covered.
    fn left_half(width: u32, height: u32) -> Mask {
        let coverage = (0..height)
            .flat_map(|_| (0..width).map(move |column| if column < width / 2 { 255 } else { 0 }))
            .collect();
        Mask::new(width, height, coverage).expect("sized")
    }

    #[test]
    fn the_subject_keeps_what_the_mask_covers() {
        let subject = subject(&opaque(40, 20), &left_half(40, 20));
        assert_eq!((subject.width(), subject.height()), (40, 20));
        let pixel = |x: u32| subject.pixel(x, 10).expect("inside");
        assert_eq!(pixel(2).alpha(), 255);
        assert_eq!(pixel(2).red(), 200);
        assert_eq!(pixel(37).alpha(), 0);
        assert_eq!(pixel(37).red(), 0, "premultiplied");
    }

    #[test]
    fn a_small_mask_covers_the_background_as_the_picture_did() {
        let mask = left_half(4, 2);
        let wide = subject(&opaque(400, 200), &mask);
        assert_eq!(wide.pixel(50, 100).map(|pixel| pixel.alpha()), Some(255));
        assert_eq!(wide.pixel(350, 100).map(|pixel| pixel.alpha()), Some(0));
        let square = subject(&opaque(200, 200), &mask);
        assert_eq!(
            square.pixel(30, 100).map(|pixel| pixel.alpha()),
            Some(255),
            "cropped at the sides, the left still covered"
        );
        assert_eq!(square.pixel(150, 100).map(|pixel| pixel.alpha()), Some(0));
    }

    #[test]
    fn masks_decode_from_grayscale_and_refuse_bad_sizes() {
        assert!(Mask::new(2, 2, vec![0; 3]).is_none());
        assert!(Mask::new(0, 0, Vec::new()).is_none());
        let mut bytes = Vec::new();
        image::GrayImage::from_pixel(3, 2, image::Luma([255]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("encodes");
        let mask = Mask::decode(&bytes).expect("decodes");
        assert_eq!((mask.width(), mask.height()), (3, 2));
        assert_eq!(mask.sample(1.5, 0.5), 255.0);
        assert!(Mask::decode(b"not a png").is_err());
    }

    #[test]
    fn occlusion_measures_how_much_of_the_clock_hides() {
        let subject = subject(&opaque(40, 20), &left_half(40, 20));
        let rect = |x, w| IntRect::from_xywh(x, 0, w, 20).expect("rect");
        assert_eq!(occlusion(&subject, rect(0, 20)), 1.0);
        assert_eq!(occlusion(&subject, rect(20, 20)), 0.0);
        assert!((occlusion(&subject, rect(10, 20)) - 0.5).abs() < 1e-6);
        assert!((occlusion(&subject, rect(30, 20)) - 0.0).abs() < 1e-6);
        assert!((occlusion(&subject, rect(-20, 40)) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_clock_is_measured_where_the_zoomed_subject_lands() {
        let subject = subject(&opaque(40, 20), &left_half(40, 20));
        let output = [400.0, 200.0];
        let clock = |x| Rect {
            x,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        };
        assert_eq!(clock_occlusion(&subject, clock(0.0), output, 1.0), 1.0);
        assert!((clock_occlusion(&subject, clock(100.0), output, 1.0) - 0.5).abs() < 1e-6);
        assert!(hides_clock(&subject, clock(100.0), output, 1.0));
        assert!(!hides_clock(&subject, clock(150.0), output, 1.0));
        let zoomed = clock_occlusion(&subject, clock(200.0), output, 2.0);
        assert_eq!(
            zoomed, 0.0,
            "a zoom about the centre keeps the right half clear"
        );
        let spread = clock_occlusion(&subject, clock(150.0), output, 2.0);
        assert!(spread > clock_occlusion(&subject, clock(150.0), output, 1.0));
    }
}

use image::{
    GrayImage, Luma, RgbImage,
    imageops::{self, FilterType},
};

use crate::{
    Plane,
    util::{guided_filter::guided_filter, otsu, smoothstep::smoothstep},
};

/// The mask's longer side: sharp enough once the host scales it to a
/// display, and the size the guided filter's radius is tuned for.
pub const LONG_SIDE: u32 = 1280;
const REC709_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// Half the soft band, in normalised depth, eased across around the cut.
const SOFT_EDGE: f32 = 8.0 / 255.0;
const GUIDE_RADIUS: usize = 8;
const GUIDE_EPSILON: f32 = 1e-3;

/// The picture's luminance in `0..=1`, whose edges the mask snaps to.
pub fn guide(picture: &RgbImage) -> Plane {
    Plane::from_fn(picture.width(), picture.height(), |x, y| {
        let rgb = picture.get_pixel(x, y).0;
        let luma: f32 = rgb
            .iter()
            .zip(REC709_LUMA)
            .map(|(&channel, weight)| f32::from(channel) * weight)
            .sum();
        Luma([luma / 255.0])
    })
}

/// 255 over what stands nearest, 0 behind it, at the guide's size: the
/// depth split where Otsu finds the cleanest break, eased over a narrow
/// band, then snapped to the picture's edges.
pub fn cut(mut disparity: Plane, guide: &Plane) -> GrayImage {
    let (low, high) = disparity
        .iter()
        .fold((f32::MAX, f32::MIN), |(low, high), &depth| {
            (low.min(depth), high.max(depth))
        });
    let range = (high - low).max(f32::EPSILON);
    disparity
        .iter_mut()
        .for_each(|depth| *depth = (*depth - low) / range);
    let Some(threshold) = otsu::threshold(&disparity) else {
        return GrayImage::new(guide.width(), guide.height());
    };
    disparity.iter_mut().for_each(|depth| {
        *depth = smoothstep(threshold - SOFT_EDGE, threshold + SOFT_EDGE, *depth)
    });
    let coarse = imageops::resize(
        &disparity,
        guide.width(),
        guide.height(),
        FilterType::Triangle,
    );
    drop(disparity);
    let refined = guided_filter(
        guide,
        &coarse,
        guide.width() as usize,
        GUIDE_RADIUS,
        GUIDE_EPSILON,
    );
    let mut mask = GrayImage::new(guide.width(), guide.height());
    mask.iter_mut()
        .zip(refined)
        .for_each(|(coverage, refined)| *coverage = (refined * 255.0).round() as u8);
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_blob_becomes_the_subject() {
        let near = |x: u32, y: u32| (x as f32 - 48.0).hypot(y as f32 - 32.0) < 12.0;
        let disparity = Plane::from_fn(96, 64, |x, y| Luma([if near(x, y) { 5.0 } else { 1.0 }]));
        let flat = Plane::from_pixel(192, 128, Luma([0.5]));
        let mask = cut(disparity, &flat);
        assert_eq!(mask.dimensions(), (192, 128));
        assert!(mask.get_pixel(96, 64).0[0] > 240);
        assert!(mask.get_pixel(10, 10).0[0] < 15);
    }

    #[test]
    fn a_flat_depth_has_no_subject() {
        let mask = cut(Plane::from_pixel(70, 40, Luma([2.0])), &Plane::new(140, 80));
        assert!(mask.iter().all(|&coverage| coverage == 0));
    }

    #[test]
    fn the_guide_is_luminance() {
        let picture = RgbImage::from_pixel(2, 2, image::Rgb([255, 255, 255]));
        assert!((guide(&picture).get_pixel(1, 1).0[0] - 1.0).abs() < 1e-5);
    }
}

use tiny_skia::Pixmap;

const PASSES: usize = 3;

/// Approximates a gaussian of about `radius` pixels with three box blurs,
/// on premultiplied pixels, in place.
pub fn box_blur(pixmap: &mut Pixmap, radius: usize) {
    if radius == 0 {
        return;
    }
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    let data = pixmap.data_mut();
    let mut scratch = vec![0u8; width.max(height) * 4];
    for _ in 0..PASSES {
        for row in 0..height {
            blur_line(data, row * width * 4, 4, width, radius, &mut scratch);
        }
        for column in 0..width {
            blur_line(data, column * 4, width * 4, height, radius, &mut scratch);
        }
    }
}

/// One running-sum box blur along `len` pixels starting at `start`, `stride`
/// bytes apart, with edges clamped.
fn blur_line(
    data: &mut [u8],
    start: usize,
    stride: usize,
    len: usize,
    radius: usize,
    scratch: &mut [u8],
) {
    let window = (2 * radius + 1) as u32;
    let at = |index: usize| start + index.min(len - 1) * stride;
    let mut sums = [0u32; 4];
    for offset in 0..=2 * radius {
        let index = at(offset.saturating_sub(radius));
        for (sum, byte) in sums.iter_mut().zip(&data[index..index + 4]) {
            *sum += u32::from(*byte);
        }
    }
    for index in 0..len {
        for (channel, sum) in sums.iter().enumerate() {
            scratch[index * 4 + channel] = (sum / window) as u8;
        }
        let leaving = at(index.saturating_sub(radius));
        let entering = at(index + radius + 1);
        for (channel, sum) in sums.iter_mut().enumerate() {
            *sum = *sum + u32::from(data[entering + channel]) - u32::from(data[leaving + channel]);
        }
    }
    for index in 0..len {
        let target = start + index * stride;
        data[target..target + 4].copy_from_slice(&scratch[index * 4..index * 4 + 4]);
    }
}

#[cfg(test)]
mod tests {
    use tiny_skia::{Color, Paint, Rect, Transform};

    use super::*;

    #[test]
    fn a_blur_spreads_a_dot_and_keeps_flat_areas_flat() {
        let mut pixmap = Pixmap::new(21, 21).expect("size");
        pixmap.fill(Color::from_rgba8(10, 20, 30, 255));
        let mut paint = Paint::default();
        paint.set_color_rgba8(255, 255, 255, 255);
        pixmap.fill_rect(
            Rect::from_xywh(10.0, 10.0, 1.0, 1.0).expect("rect"),
            &paint,
            Transform::identity(),
            None,
        );
        box_blur(&mut pixmap, 2);
        let pixel = |x: u32, y: u32| pixmap.pixel(x, y).expect("inside");
        assert!(pixel(10, 10).red() < 255);
        assert!(pixel(12, 10).red() > 10);
        assert_eq!(pixel(0, 0).red(), 10);
        assert_eq!(pixel(0, 0).alpha(), 255);
    }
}

use tiny_skia::Pixmap;

const PASSES: usize = 3;

/// Approximates a gaussian of about `radius` pixels with three box blurs,
/// on premultiplied pixels, in place.
pub fn box_blur(pixmap: &mut Pixmap, radius: usize) {
    blur_passes(pixmap, [radius; PASSES]);
}

/// Blurs premultiplied `pixmap` in place like a gaussian of `sigma` pixels,
/// with three box blurs of [`box_radii`].
pub fn gaussian_blur(pixmap: &mut Pixmap, sigma: f32) {
    blur_passes(pixmap, box_radii(sigma));
}

/// The radii of three box blurs that together blur like a gaussian of
/// `sigma` pixels, as nearly as whole radii can: a box of radius `r` adds
/// `r(r + 1) / 3` of variance, so some passes take one pixel more.
pub fn box_radii(sigma: f32) -> [usize; PASSES] {
    let variance = sigma.max(0.0).powi(2);
    let box_variance = |radius: usize| (radius * (radius + 1)) as f32 / 3.0;
    let per_pass = variance / PASSES as f32;
    let narrow = ((3.0 * per_pass + 0.25).sqrt() - 0.5).floor() as usize;
    let missing = variance - PASSES as f32 * box_variance(narrow);
    let wider = (missing / (box_variance(narrow + 1) - box_variance(narrow))).round() as usize;
    std::array::from_fn(|pass| narrow + usize::from(pass < wider))
}

fn blur_passes(pixmap: &mut Pixmap, radii: [usize; PASSES]) {
    if radii == [0; PASSES] {
        return;
    }
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    let data = pixmap.data_mut();
    let mut scratch = vec![0u8; width.max(height) * 4];
    for radius in radii.into_iter().filter(|radius| *radius > 0) {
        for row in 0..height {
            blur_line(data, row * width * 4, 4, width, radius, &mut scratch);
        }
        for column in 0..width {
            blur_line(data, column * 4, width * 4, height, radius, &mut scratch);
        }
    }
}

/// One running-sum box blur along `len` pixels starting at `start`, `stride`
/// bytes apart, with edges clamped and averages rounded, so passes never
/// darken.
fn blur_line(
    data: &mut [u8],
    start: usize,
    stride: usize,
    len: usize,
    radius: usize,
    scratch: &mut [u8],
) {
    let window = (2 * radius + 1) as u32;
    let half_window = radius as u32;
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
            scratch[index * 4 + channel] = ((sum + half_window) / window) as u8;
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

    #[test]
    fn blurring_keeps_the_brightness() {
        let mut pixmap = Pixmap::new(64, 64).expect("size");
        for (index, pixel) in pixmap.data_mut().chunks_exact_mut(4).enumerate() {
            let level = (index * 37 % 101 + 77) as u8;
            pixel.copy_from_slice(&[level, level, level, u8::MAX]);
        }
        let mean = |pixmap: &Pixmap| {
            let data = pixmap.data();
            data.iter()
                .step_by(4)
                .map(|red| f32::from(*red))
                .sum::<f32>()
                / (data.len() / 4) as f32
        };
        let before = mean(&pixmap);
        gaussian_blur(&mut pixmap, 6.0);
        assert!(
            (mean(&pixmap) - before).abs() < 0.25,
            "{before} {}",
            mean(&pixmap)
        );
    }

    #[test]
    fn three_boxes_add_up_to_the_gaussian() {
        assert_eq!(box_radii(0.0), [0; PASSES]);
        assert_eq!(box_radii(30.0_f32.sqrt()), [5; PASSES]);
        for sigma in [0.8, 2.2, 5.0, 12.9, 40.0] {
            let variance: f32 = box_radii(sigma)
                .iter()
                .map(|radius| (radius * (radius + 1)) as f32 / 3.0)
                .sum();
            assert!((variance.sqrt() - sigma).abs() < 0.2, "{sigma}: {variance}");
        }
    }

    #[test]
    fn a_gaussian_blur_softens_an_edge_by_its_sigma() {
        let side = 121;
        let mut pixmap = Pixmap::new(side, 1).expect("size");
        pixmap.data_mut()[..(side / 2) as usize * 4].fill(u8::MAX);
        let sigma = 6.0;
        gaussian_blur(&mut pixmap, sigma);
        let alpha = |column: u32| f32::from(pixmap.pixel(column, 0).expect("inside").alpha());
        let kernel: Vec<f32> = (0..side - 1)
            .map(|column| alpha(column) - alpha(column + 1))
            .collect();
        let total: f32 = kernel.iter().sum();
        let mean = kernel
            .iter()
            .enumerate()
            .map(|(column, weight)| column as f32 * weight)
            .sum::<f32>()
            / total;
        let variance = kernel
            .iter()
            .enumerate()
            .map(|(column, weight)| (column as f32 - mean).powi(2) * weight)
            .sum::<f32>()
            / total;
        assert!((variance.sqrt() - sigma).abs() < 0.3, "{}", variance.sqrt());
    }
}

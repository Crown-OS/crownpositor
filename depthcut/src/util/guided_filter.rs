/// `input` smoothed so it follows the edges of `guide` (He, Sun and Tang,
/// "Guided Image Filtering", 2010): within every window of `radius` the
/// result is a linear function of the guide, so a coarse mask snaps to the
/// picture's own outlines. Edges fainter than `epsilon` (a variance) are
/// smoothed away. Both planes are `width` wide, row by row, in `0..=1`.
pub fn guided_filter(
    guide: &[f32],
    input: &[f32],
    width: usize,
    radius: usize,
    epsilon: f32,
) -> Vec<f32> {
    let mean = |plane: &[f32]| box_mean(plane, width, radius);
    let product = |left: &[f32], right: &[f32]| -> Vec<f32> {
        left.iter()
            .zip(right)
            .map(|(left, right)| left * right)
            .collect()
    };
    let mean_guide = mean(guide);
    let mean_input = mean(input);
    let mean_guide_squared = mean(&product(guide, guide));
    let mean_guide_input = mean(&product(guide, input));
    let (slope, offset): (Vec<f32>, Vec<f32>) = (0..guide.len())
        .map(|index| {
            let variance = mean_guide_squared[index] - mean_guide[index] * mean_guide[index];
            let covariance = mean_guide_input[index] - mean_guide[index] * mean_input[index];
            let slope = covariance / (variance + epsilon);
            (slope, mean_input[index] - slope * mean_guide[index])
        })
        .unzip();
    mean(&slope)
        .iter()
        .zip(mean(&offset))
        .zip(guide)
        .map(|((slope, offset), guide)| (slope * guide + offset).clamp(0.0, 1.0))
        .collect()
}

/// The mean of every `2 × radius + 1` square window, clipped to the plane.
fn box_mean(plane: &[f32], width: usize, radius: usize) -> Vec<f32> {
    let area = SummedArea::new(plane, width);
    let area = &area;
    (0..area.height)
        .flat_map(|y| (0..width).map(move |x| area.window_mean(x, y, radius)))
        .collect()
}

/// Running sums from the top-left corner, one row and column of zeros
/// ahead, so any rectangle's sum takes four reads.
struct SummedArea {
    stride: usize,
    height: usize,
    sums: Vec<f64>,
}

impl SummedArea {
    fn new(plane: &[f32], width: usize) -> Self {
        let (stride, height) = (width + 1, plane.len() / width);
        let mut sums = vec![0.0; stride * (height + 1)];
        for (y, row) in plane.chunks_exact(width).enumerate() {
            let mut row_sum = 0.0;
            for (x, &value) in row.iter().enumerate() {
                row_sum += f64::from(value);
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row_sum;
            }
        }
        Self {
            stride,
            height,
            sums,
        }
    }

    fn window_mean(&self, x: usize, y: usize, radius: usize) -> f32 {
        let (left, top) = (x.saturating_sub(radius), y.saturating_sub(radius));
        let right = (x + radius + 1).min(self.stride - 1);
        let bottom = (y + radius + 1).min(self.height);
        let at = |x: usize, y: usize| self.sums[y * self.stride + x];
        let sum = at(right, bottom) - at(left, bottom) - at(right, top) + at(left, top);
        (sum / ((right - left) * (bottom - top)) as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 64;
    const HEIGHT: usize = 16;
    const EDGE: usize = 32;
    const RADIUS: usize = 4;
    const EPSILON: f32 = 1e-3;

    fn plane(value_at: impl Fn(usize) -> f32) -> Vec<f32> {
        (0..WIDTH * HEIGHT)
            .map(|index| value_at(index % WIDTH))
            .collect()
    }

    fn steepest_rise(row: &[f32]) -> (usize, f32) {
        row.windows(2)
            .map(|pair| pair[1] - pair[0])
            .enumerate()
            .fold((0, f32::MIN), |steepest, (at, rise)| {
                if rise > steepest.1 {
                    (at, rise)
                } else {
                    steepest
                }
            })
    }

    #[test]
    fn a_blurred_mask_snaps_to_the_guides_step() {
        let guide = plane(|x| if x < EDGE { 0.1 } else { 0.9 });
        let blurred = plane(|x| ((x as f32 - (EDGE - 5) as f32) / 6.0).clamp(0.0, 1.0));
        let refined = guided_filter(&guide, &blurred, WIDTH, RADIUS, EPSILON);
        let row = &refined[WIDTH * HEIGHT / 2..][..WIDTH];
        let (at, rise) = steepest_rise(row);
        assert_eq!(at, EDGE - 1, "{row:?}");
        assert!(rise > 2.0 * steepest_rise(&blurred[..WIDTH]).1, "{row:?}");
    }

    #[test]
    fn a_flat_guide_only_smooths() {
        let guide = plane(|_| 0.5);
        let step = plane(|x| if x < EDGE { 0.0 } else { 1.0 });
        let refined = guided_filter(&guide, &step, WIDTH, RADIUS, EPSILON);
        let row = &refined[..WIDTH];
        assert!((row[EDGE] - 0.5).abs() < 0.2);
        assert!(row.windows(2).all(|pair| pair[0] <= pair[1] + 1e-6));
        assert_eq!((row[0], row[WIDTH - 1]), (0.0, 1.0));
    }

    #[test]
    fn a_box_mean_clips_at_the_border() {
        let ones = plane(|_| 1.0);
        assert!(
            box_mean(&ones, WIDTH, RADIUS)
                .iter()
                .all(|&mean| (mean - 1.0).abs() < 1e-6)
        );
        let ramp = plane(|x| x as f32);
        let means = box_mean(&ramp, WIDTH, 1);
        assert_eq!((means[0], means[1], means[WIDTH - 1]), (0.5, 1.0, 62.5));
    }
}

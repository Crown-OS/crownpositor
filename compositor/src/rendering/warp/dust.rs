//! The dust's grid and the reach of its fall, in the physical pixels the
//! shader draws in.

use smithay::utils::{Physical, Rectangle};

use crate::{
    shaders::warp::DustUniforms,
    shell::dust::{DRAG, Dust, GRAIN, GRAVITY, JITTER, KICK, LIFE, Reach, SPREAD, SWEEP},
};

/// Most particles a window breaks into. A window too big for grains of the
/// usual size gets coarser ones.
const MOST_PARTICLES: f64 = 120_000.0;

/// Side of a grain on `window`, physical pixels.
fn grain(window: Rectangle<f64, Physical>, scale: f64) -> f64 {
    let grain = GRAIN * scale;
    let count = window.size.w * window.size.h / (grain * grain);
    grain * (count / MOST_PARTICLES).sqrt().max(1.0)
}

pub fn uniforms(window: Rectangle<f64, Physical>, dust: &Dust, scale: f64) -> DustUniforms {
    let grain = grain(window, scale);
    let cells = |length: f64| ((length / grain).ceil() as i32).max(1);
    let scale = scale as f32;
    DustUniforms {
        cells: [cells(window.size.w), cells(window.size.h)],
        seed: dust.seed(),
        timing: [SWEEP, JITTER, LIFE, dust.elapsed()],
        physics: [GRAVITY * scale, DRAG, SPREAD * scale, KICK * scale],
    }
}

/// Everywhere a particle of `window` can be drawn.
pub fn bounds(window: Rectangle<f64, Physical>, scale: f64) -> Rectangle<i32, Physical> {
    let reach = Reach::of_fall();
    let grain = grain(window, scale);
    let out = |length: f32| f64::from(length) * scale + grain;
    let (left, top) = (
        window.loc.x - out(reach.sideways),
        window.loc.y - out(reach.up),
    );
    let (right, bottom) = (
        window.loc.x + window.size.w + out(reach.sideways),
        window.loc.y + window.size.h + out(reach.down),
    );
    Rectangle::from_extremities(
        (left.floor() as i32, top.floor() as i32),
        (right.ceil() as i32, bottom.ceil() as i32),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(w: f64, h: f64) -> Rectangle<f64, Physical> {
        Rectangle::new((100.0, 100.0).into(), (w, h).into())
    }

    #[test]
    fn a_window_breaks_into_grains_of_four_logical_pixels() {
        let dust = uniforms(window(800.0, 600.0), &Dust::new(1), 1.0);
        assert_eq!(dust.cells, [200, 150]);
    }

    #[test]
    fn a_huge_window_gets_coarser_grains() {
        let dust = uniforms(window(3840.0, 2160.0), &Dust::new(1), 2.0);
        let count = f64::from(dust.cells[0] * dust.cells[1]);
        assert!(count <= MOST_PARTICLES * 1.02, "{count} particles");
    }

    #[test]
    fn the_fall_reaches_well_below_the_window() {
        let window = window(800.0, 600.0);
        let bounds = bounds(window, 1.25).to_f64();
        assert!(bounds.contains_rect(window));
        assert!(bounds.loc.y + bounds.size.h > window.loc.y + window.size.h + 150.0);
    }
}

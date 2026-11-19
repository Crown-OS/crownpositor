//! A closed window dissolving into dust that the air carries away.
//!
//! The dissolve sweeps in from the close control's edge, and each grain it
//! frees is caught by a steady wind that streams it up and away, weaving as
//! it goes, while it shrinks and fades.
//!
//! The shell only keeps the clock. Each grain's flight is closed-form, its
//! velocity relaxing from a random scatter toward the wind's under linear
//! drag, so the GPU works it out per vertex from these constants and the
//! elapsed time, and nothing is stored per grain.

/// Seconds the dissolve takes to sweep across the window.
pub const SWEEP: f32 = 0.36;
/// Seconds of random hold-off on top of the sweep, so the edge is ragged.
pub const JITTER: f32 = 0.12;
/// Seconds a grain drifts, shrinking and fading, before it is gone.
pub const LIFE: f32 = 0.7;
/// The speed the air carries a grain at, logical pixels per second: away
/// from the window's right edge and up.
pub const WIND: [f32; 2] = [560.0, -220.0];
/// Linear drag, per second: how quickly a freed grain is caught by the wind.
pub const DRAG: f32 = 3.2;
/// Largest speed a grain breaks away with, either way on each axis, logical
/// pixels per second.
pub const SCATTER: f32 = 140.0;
/// How far a grain weaves across the stream by the end of its life, as a
/// share of `SCATTER` in pixels.
pub const WEAVE: f32 = 0.25;
/// Side of a grain, logical pixels.
pub const GRAIN: f64 = 1.0;

/// How far past the window a grain can get, logical pixels: the bound of the
/// closed form over a grain's life, its weave included.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    pub left: f32,
    pub right: f32,
    pub up: f32,
    pub down: f32,
}

impl Reach {
    pub fn of_flow() -> Self {
        let caught = (1.0 - (-DRAG * LIFE).exp()) / DRAG;
        let carried = LIFE - caught;
        let weave = SCATTER * WEAVE;
        let extent = |wind: f32| {
            let back = SCATTER * caught - wind.min(0.0) * carried + weave;
            let ahead = SCATTER * caught + wind.max(0.0) * carried + weave;
            (back, ahead)
        };
        let (left, right) = extent(WIND[0]);
        let (up, down) = extent(WIND[1]);
        Self {
            left,
            right,
            up,
            down,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dust {
    elapsed: f32,
    seed: u32,
}

impl Dust {
    pub fn new(seed: u64) -> Self {
        Self {
            elapsed: 0.0,
            seed: (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32,
        }
    }

    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// Advances the clock; returns whether any grain is still on screen.
    pub fn step(&mut self, dt: f32) -> bool {
        self.elapsed += dt;
        self.elapsed < SWEEP + JITTER + LIFE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader's closed form without the weave: where a grain freed with
    /// `launch` is `flight` seconds later, relative to where it started; y
    /// grows down.
    fn displacement(launch: (f32, f32), flight: f32) -> (f32, f32) {
        let caught = (1.0 - (-DRAG * flight).exp()) / DRAG;
        (
            WIND[0] * flight + (launch.0 - WIND[0]) * caught,
            WIND[1] * flight + (launch.1 - WIND[1]) * caught,
        )
    }

    #[test]
    fn it_settles_within_about_a_second() {
        let mut dust = Dust::new(7);
        let mut frames = 0;
        while dust.step(1.0 / 120.0) {
            frames += 1;
            assert!(frames < 150, "the dust never settled");
        }
        assert!(frames > 100);
    }

    #[test]
    fn no_grain_leaves_its_reach() {
        let reach = Reach::of_flow();
        let weave = SCATTER * WEAVE;
        for x in [-SCATTER, 0.0, SCATTER] {
            for y in [-SCATTER, 0.0, SCATTER] {
                for step in 0..=100 {
                    let (dx, dy) = displacement((x, y), LIFE * step as f32 / 100.0);
                    assert!(-dx + weave <= reach.left + 1e-3, "{dx} left");
                    assert!(dx + weave <= reach.right + 1e-3, "{dx} right");
                    assert!(-dy + weave <= reach.up + 1e-3, "{dy} up");
                    assert!(dy + weave <= reach.down + 1e-3, "{dy} down");
                }
            }
        }
    }

    #[test]
    fn every_grain_is_blown_up_and_away() {
        for x in [-SCATTER, SCATTER] {
            for y in [-SCATTER, SCATTER] {
                let (dx, dy) = displacement((x, y), LIFE);
                assert!(dx > 150.0, "only carried {dx} away");
                assert!(dy < 0.0, "sank {dy}");
            }
        }
    }

    #[test]
    fn different_windows_dissolve_differently() {
        assert_ne!(Dust::new(1).seed(), Dust::new(2).seed());
    }
}

//! A closed window crumbling into particles that fall away under gravity.
//!
//! The shell only keeps the clock. Each particle's flight is closed-form —
//! ballistic under gravity against linear drag — so the GPU works it out per
//! vertex from these constants and the elapsed time, and nothing is stored
//! per particle.

/// Seconds the crumbling takes to sweep from the bottom right to the top left.
pub const SWEEP: f32 = 0.38;
/// Seconds of random hold-off on top of the sweep, so the edge is ragged.
pub const JITTER: f32 = 0.1;
/// Seconds a particle falls, shrinking and fading, before it is gone.
pub const LIFE: f32 = 0.62;
/// Logical pixels per second squared.
pub const GRAVITY: f32 = 2400.0;
/// Linear drag, per second: a falling particle tends to `GRAVITY / DRAG`.
pub const DRAG: f32 = 2.4;
/// Largest sideways launch speed, logical pixels per second.
pub const SPREAD: f32 = 110.0;
/// Largest upward launch speed, logical pixels per second.
pub const KICK: f32 = 150.0;
/// Side of a particle, logical pixels.
pub const GRAIN: f64 = 4.0;

/// How far past the window a particle can get, logical pixels: the bound of
/// the closed form over a particle's life.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    pub sideways: f32,
    pub up: f32,
    pub down: f32,
}

impl Reach {
    pub fn of_fall() -> Self {
        let settled = 1.0 - (-DRAG * LIFE).exp();
        let terminal = GRAVITY / DRAG;
        Self {
            sideways: SPREAD * settled / DRAG,
            up: KICK / DRAG,
            down: terminal * LIFE - terminal * settled / DRAG,
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

    /// Advances the clock; returns whether any particle is still on screen.
    pub fn step(&mut self, dt: f32) -> bool {
        self.elapsed += dt;
        self.elapsed < SWEEP + JITTER + LIFE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader's closed form: where a particle released with `launch` is
    /// `flight` seconds later, relative to where it started; y grows down.
    fn displacement(launch: (f32, f32), flight: f32) -> (f32, f32) {
        let settled = (1.0 - (-DRAG * flight).exp()) / DRAG;
        let terminal = GRAVITY / DRAG;
        (
            launch.0 * settled,
            terminal * flight + (launch.1 - terminal) * settled,
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
    fn no_particle_leaves_its_reach() {
        let reach = Reach::of_fall();
        let launches = [
            (SPREAD, -KICK),
            (-SPREAD, -KICK),
            (SPREAD, 0.0),
            (-SPREAD, 0.0),
            (0.0, 0.0),
        ];
        for launch in launches {
            for step in 0..=100 {
                let (x, y) = displacement(launch, LIFE * step as f32 / 100.0);
                assert!(x.abs() <= reach.sideways + 1e-3, "{x} sideways");
                assert!(-y <= reach.up + 1e-3, "{y} up");
                assert!(y <= reach.down + 1e-3, "{y} down");
            }
        }
    }

    #[test]
    fn particles_fall() {
        let (_, y) = displacement((0.0, -KICK), LIFE);
        assert!(y > 100.0, "only fell {y}");
    }

    #[test]
    fn different_windows_crumble_differently() {
        assert_ne!(Dust::new(1).seed(), Dust::new(2).seed());
    }
}

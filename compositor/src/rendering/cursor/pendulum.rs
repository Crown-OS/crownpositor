//! A drag icon hanging from the pointer like a badge on a lanyard: held where
//! it was grabbed, its middle swings under its own momentum.
//!
//! The middle is a Verlet point mass on a rigid lever to the pointer. Moving
//! the pointer drags it along the lever and the mass keeps the speed it was
//! dragged with, so a flick swings the icon and it rocks back to hang the way
//! it was picked up.

use smithay::utils::{Logical, Point};

/// Fixed integrator step: Verlet's momentum term assumes equal steps.
const SUBSTEP: f64 = 1.0 / 240.0;
/// The most time one frame may simulate, so a stall cannot fling the icon.
const MAX_DT: f64 = 1.0 / 30.0;
/// Logical pixels per second squared. A 48 px lever swings about once a
/// second.
const GRAVITY: f64 = 2400.0;
/// Swing speed lost per second. Little, so a swing carries on for a while.
const DRAG: f64 = 1.5;
/// An icon grabbed near its middle would hang from no lever at all and never
/// swing, so it hangs from at least this much.
const MIN_LEVER: f64 = 24.0;
/// Below these the swing is invisible and frames can stop.
const REST_ANGLE: f64 = 0.01;
const REST_SPEED: f64 = 0.01;

pub struct Pendulum {
    pivot: Point<f64, Logical>,
    bob: Point<f64, Logical>,
    previous: Point<f64, Logical>,
    /// Unit direction from pivot to bob when the icon hangs unrotated.
    hang: Point<f64, Logical>,
    lever: f64,
    middle: Point<f64, Logical>,
    unspent: f64,
}

impl Pendulum {
    /// Hangs at rest from `pivot`, its middle `middle` away from it.
    pub fn new(pivot: Point<f64, Logical>, middle: Point<f64, Logical>) -> Self {
        let (hang, lever) = lever_to(middle);
        let bob = pivot + hang.upscale(lever);
        Self {
            pivot,
            bob,
            previous: bob,
            hang,
            lever,
            middle,
            unspent: 0.0,
        }
    }

    /// Follows the pointer to `pivot`, `dt` seconds on, and returns how far the
    /// icon has swung, clockwise in radians. `middle` is where the unrotated
    /// icon's middle is from the pivot; a new one rehangs the icon at the same
    /// swing.
    pub fn swing(
        &mut self,
        pivot: Point<f64, Logical>,
        middle: Point<f64, Logical>,
        dt: f64,
    ) -> f64 {
        if middle != self.middle {
            self.rehang(middle);
        }
        self.unspent = (self.unspent + dt).min(MAX_DT);
        let steps = (self.unspent / SUBSTEP).floor();
        if steps >= 1.0 {
            let from = self.pivot;
            let carried = (pivot - from).downscale(steps);
            for step in 1..=steps as u32 {
                self.step(from + carried.upscale(f64::from(step)), carried);
            }
            self.unspent -= steps * SUBSTEP;
            self.pivot = pivot;
        }
        if self.at_rest() {
            self.bob = self.pivot + self.hang.upscale(self.lever);
            self.previous = self.bob;
        }
        self.angle()
    }

    pub fn at_rest(&self) -> bool {
        self.angle().abs() < REST_ANGLE && distance(self.bob - self.previous) < REST_SPEED
    }

    /// Only the swing about the pointer is damped, not the pointer's own
    /// motion: a steady drag carries the icon along hanging straight.
    fn step(&mut self, pivot: Point<f64, Logical>, carried: Point<f64, Logical>) {
        let swing = (self.bob - self.previous - carried).upscale((-DRAG * SUBSTEP).exp());
        let fallen = self.bob + carried + swing + self.hang.upscale(GRAVITY * SUBSTEP * SUBSTEP);
        self.previous = self.bob;
        self.bob = pivot
            + unit(fallen - pivot)
                .unwrap_or(self.hang)
                .upscale(self.lever);
    }

    fn angle(&self) -> f64 {
        let arm = self.bob - self.pivot;
        let cross = self.hang.x * arm.y - self.hang.y * arm.x;
        let dot = self.hang.x * arm.x + self.hang.y * arm.y;
        cross.atan2(dot)
    }

    fn rehang(&mut self, middle: Point<f64, Logical>) {
        let angle = self.angle();
        (self.hang, self.lever) = lever_to(middle);
        self.middle = middle;
        self.bob = self.pivot + rotate(self.hang, angle).upscale(self.lever);
        self.previous = self.bob;
    }
}

/// Turns `point` clockwise on screen by `angle` radians about the origin.
pub fn rotate<Kind>(point: Point<f64, Kind>, angle: f64) -> Point<f64, Kind> {
    let (sin, cos) = angle.sin_cos();
    Point::from((point.x * cos - point.y * sin, point.x * sin + point.y * cos))
}

fn lever_to(middle: Point<f64, Logical>) -> (Point<f64, Logical>, f64) {
    let down = Point::from((0.0, 1.0));
    let hang = unit(middle)
        .filter(|_| distance(middle) >= 1.0)
        .unwrap_or(down);
    (hang, distance(middle).max(MIN_LEVER))
}

fn distance(vector: Point<f64, Logical>) -> f64 {
    vector.x.hypot(vector.y)
}

fn unit(vector: Point<f64, Logical>) -> Option<Point<f64, Logical>> {
    let length = distance(vector);
    (length > f64::EPSILON).then(|| vector.downscale(length))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f64 = 1.0 / 60.0;

    fn hanging() -> Pendulum {
        Pendulum::new(Point::default(), Point::from((0.0, 40.0)))
    }

    #[test]
    fn an_icon_left_alone_stays_as_it_was_picked_up() {
        let mut pendulum = hanging();
        for _ in 0..60 {
            assert!(
                pendulum
                    .swing(Point::default(), Point::from((0.0, 40.0)), FRAME)
                    .abs()
                    < 1e-9
            );
        }
        assert!(pendulum.at_rest());
    }

    #[test]
    fn dragging_right_swings_the_icon_back_to_the_left() {
        let mut pendulum = hanging();
        let angle = (1..=6)
            .map(|frame| {
                let pivot = Point::from((f64::from(frame) * 20.0, 0.0));
                pendulum.swing(pivot, Point::from((0.0, 40.0)), FRAME)
            })
            .last()
            .unwrap_or_default();
        assert!(angle > 0.1, "only swung {angle}");
    }

    #[test]
    fn a_flick_swings_past_rest_and_then_settles() {
        let mut pendulum = hanging();
        let middle = Point::from((0.0, 40.0));
        for frame in 1..=6 {
            pendulum.swing(Point::from((f64::from(frame) * 20.0, 0.0)), middle, FRAME);
        }
        let stop = Point::from((120.0, 0.0));
        let swings: Vec<f64> = (0..60)
            .map(|_| pendulum.swing(stop, middle, FRAME))
            .collect();
        assert!(
            swings.iter().any(|angle| *angle < -0.05),
            "never swung through"
        );
        let settled = (0..900).any(|_| {
            pendulum.swing(stop, middle, FRAME);
            pendulum.at_rest()
        });
        assert!(settled, "still swinging after 15 s");
    }

    #[test]
    fn a_steady_drag_carries_the_icon_hanging_straight() {
        let mut pendulum = hanging();
        let middle = Point::from((0.0, 40.0));
        let angle = (1..=600)
            .map(|frame| {
                let pivot = Point::from((f64::from(frame) * 10.0, 0.0));
                pendulum.swing(pivot, middle, FRAME)
            })
            .last()
            .unwrap_or_default();
        assert!(angle.abs() < 0.02, "leans {angle} at a steady 600 px/s");
    }

    #[test]
    fn a_grab_at_the_middle_still_hangs_down() {
        let pendulum = Pendulum::new(Point::default(), Point::default());
        assert_eq!(pendulum.hang, Point::from((0.0, 1.0)));
        assert_eq!(pendulum.lever, MIN_LEVER);
    }

    #[test]
    fn rehanging_keeps_the_swing() {
        let mut pendulum = hanging();
        for frame in 1..=4 {
            pendulum.swing(
                Point::from((f64::from(frame) * 20.0, 0.0)),
                Point::from((0.0, 40.0)),
                FRAME,
            );
        }
        let before = pendulum.angle();
        pendulum.rehang(Point::from((10.0, 30.0)));
        assert!((pendulum.angle() - before).abs() < 1e-9);
    }

    #[test]
    fn a_stalled_frame_cannot_fling_the_icon() {
        let mut pendulum = hanging();
        let angle = pendulum.swing(Point::from((5000.0, 0.0)), Point::from((0.0, 40.0)), 10.0);
        assert!(angle.is_finite());
        assert!(distance(pendulum.bob - pendulum.pivot) - pendulum.lever < 1e-6);
    }
}

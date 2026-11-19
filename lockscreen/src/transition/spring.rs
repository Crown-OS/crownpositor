use std::{
    f32::consts::{PI, TAU},
    time::{Duration, Instant},
};

/// Seconds one undamped swing of the lock's spring would take.
const RESPONSE: f32 = 0.38;
/// Slightly under critical: a move overshoots by about 1 % and lands softly,
/// about half a second after it starts.
const DAMPING: f32 = 0.82;
/// How far from its target, in whole moves, a spring counts as landed: less
/// than an 8-bit channel can show.
const LANDED: f32 = 1.0 / 512.0;
/// The clock and subject rising in: a slow glide that barely overshoots.
const RISE_RESPONSE: f32 = 0.7;
const RISE_DAMPING: f32 = 0.86;
/// The background flooding out behind them: slower still and closer to
/// critical, so it pours rather than snaps.
const FLOOD_RESPONSE: f32 = 1.4;
const FLOOD_DAMPING: f32 = 0.9;

/// A damped spring, described by its response and damping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    /// Seconds one undamped swing would take.
    pub response: f32,
    /// How quickly the swinging dies away; kept below 1 so a move overshoots
    /// a touch before it settles.
    pub damping: f32,
}

impl Spring {
    /// Every move the lock screen makes.
    pub const LOCK: Self = Self {
        response: RESPONSE,
        damping: DAMPING,
    };

    /// The lock arriving: its clock and subject rising in over the desktop.
    pub const RISE: Self = Self {
        response: RISE_RESPONSE,
        damping: RISE_DAMPING,
    };

    /// The lock's background flooding out from behind its subject.
    pub const FLOOD: Self = Self {
        response: FLOOD_RESPONSE,
        damping: FLOOD_DAMPING,
    };

    /// How far a move from 0 to 1 has got `elapsed` after it started:
    /// exactly 1 once it has landed.
    pub fn progress(self, elapsed: Duration) -> f32 {
        1.0 + self.solve(-1.0, 0.0, elapsed.as_secs_f32()).0
    }

    /// How long a move from 0 to 1 takes to land.
    pub fn settle_time(self) -> Duration {
        Duration::from_secs_f32(self.landing(-1.0, 0.0))
    }

    /// How long a move from 0 to 1 takes to first reach 1, where it starts
    /// to overshoot.
    pub fn arrival_time(self) -> Duration {
        let swing = self.swing();
        Duration::from_secs_f32((PI - (swing / self.decay()).atan()) / swing)
    }

    fn decay(self) -> f32 {
        self.damping * TAU / self.response
    }

    fn swing(self) -> f32 {
        TAU / self.response * (1.0 - self.damping * self.damping).sqrt()
    }

    /// The cosine and sine weights of a move that leaves `displacement`
    /// from its target at `velocity`.
    fn weights(self, displacement: f32, velocity: f32) -> [f32; 2] {
        [
            displacement,
            (velocity + self.decay() * displacement) / self.swing(),
        ]
    }

    /// Seconds until a move that leaves `displacement` from its target at
    /// `velocity` stays within [`LANDED`] of it.
    fn landing(self, displacement: f32, velocity: f32) -> f32 {
        let [cosine, sine] = self.weights(displacement, velocity);
        ((cosine.hypot(sine) / LANDED).ln() / self.decay()).max(0.0)
    }

    /// Displacement from the target and velocity `time` seconds into a move
    /// that left `displacement` at `velocity`; both zero once it has landed.
    fn solve(self, displacement: f32, velocity: f32, time: f32) -> (f32, f32) {
        if time >= self.landing(displacement, velocity) {
            return (0.0, 0.0);
        }
        let [cosine, sine] = self.weights(displacement, velocity);
        let (decay, swing) = (self.decay(), self.swing());
        let envelope = (-decay * time).exp();
        let (sin, cos) = (swing * time).sin_cos();
        let place = envelope * (cosine * cos + sine * sin);
        let speed = envelope
            * ((sine * swing - decay * cosine) * cos - (cosine * swing + decay * sine) * sin);
        (place, speed)
    }
}

/// A value a spring carries toward its target. Solved in closed form, so it
/// reads the same at any instant, and sent elsewhere mid-move it carries on
/// from where it is at the speed it has, without a jump.
#[derive(Debug, Clone, Copy)]
pub struct SpringValue {
    spring: Spring,
    target: f32,
    /// How far from `target` the value was at `since`, and how fast it went.
    displacement: f32,
    velocity: f32,
    since: Instant,
}

impl SpringValue {
    pub fn resting(value: f32, now: Instant) -> Self {
        Self::moving(value, value, now)
    }

    /// At `from` and still, about to head for `target`.
    pub fn moving(from: f32, target: f32, now: Instant) -> Self {
        Self {
            spring: Spring::LOCK,
            target,
            displacement: from - target,
            velocity: 0.0,
            since: now,
        }
    }

    /// Carried by `spring` instead of [`Spring::LOCK`].
    pub fn on(self, spring: Spring) -> Self {
        Self { spring, ..self }
    }

    pub fn value(&self, now: Instant) -> f32 {
        self.target + self.state(now).0
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Heads for `target` from wherever the value is at `now`, keeping its
    /// speed.
    pub fn retarget(&mut self, target: f32, now: Instant) {
        let (displacement, velocity) = self.state(now);
        *self = Self {
            target,
            displacement: self.target + displacement - target,
            velocity,
            since: now,
            ..*self
        };
    }

    /// Whether it has set off by `now`: one made to start later has not.
    pub fn has_started(&self, now: Instant) -> bool {
        now >= self.since
    }

    pub fn is_settled(&self, now: Instant) -> bool {
        self.elapsed(now) >= self.spring.landing(self.displacement, self.velocity)
    }

    fn elapsed(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.since).as_secs_f32()
    }

    fn state(&self, now: Instant) -> (f32, f32) {
        self.spring
            .solve(self.displacement, self.velocity, self.elapsed(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn a_move_lands_in_about_half_a_second_after_a_slight_overshoot() {
        let spring = Spring::LOCK;
        assert_eq!(spring.progress(Duration::ZERO), 0.0);
        let settle = spring.settle_time();
        assert!(
            (Duration::from_millis(450)..=Duration::from_millis(550)).contains(&settle),
            "{settle:?}"
        );
        assert_eq!(spring.progress(settle), 1.0);
        let peak = (0..500)
            .map(|millis| spring.progress(Duration::from_millis(millis)))
            .fold(0.0_f32, f32::max);
        assert!(peak > 1.0 && peak < 1.03, "overshoots to {peak}");
    }

    #[test]
    fn the_flood_pours_in_slower_and_softer() {
        let (lock, flood) = (Spring::LOCK, Spring::FLOOD);
        assert!(flood.settle_time() > lock.settle_time());
        assert!(flood.settle_time() < Duration::from_secs(2));
        let peak = (0..2000)
            .map(|millis| flood.progress(Duration::from_millis(millis)))
            .fold(0.0_f32, f32::max);
        assert!(peak > 1.0 && peak < 1.01, "overshoots to {peak}");
    }

    #[test]
    fn a_rise_arrives_where_it_starts_to_overshoot() {
        let rise = Spring::RISE;
        let arrival = rise.arrival_time();
        assert!(arrival < rise.settle_time());
        assert!(rise.progress(arrival - Duration::from_millis(5)) < 1.0);
        assert!(rise.progress(arrival + Duration::from_millis(5)) > 1.0);
    }

    #[test]
    fn turning_back_keeps_place_and_speed() {
        let start = Instant::now();
        let mut value = SpringValue::moving(0.0, 1.0, start);
        let turn = at(start, 120);
        let (place, speed) = (value.value(turn), value.state(turn).1);
        assert!(speed > 0.0);
        value.retarget(0.0, turn);
        assert!((value.value(turn) - place).abs() < 1e-6);
        assert!((value.state(turn).1 - speed).abs() < 1e-4);
        assert!(
            value.value(at(turn, 16)) > place,
            "it carries on before turning"
        );
        assert!(value.is_settled(at(turn, 2000)));
        assert_eq!(value.value(at(turn, 2000)), 0.0);
    }

    #[test]
    fn a_resting_value_stays_put() {
        let start = Instant::now();
        let value = SpringValue::resting(1.0, start);
        assert!(value.is_settled(start));
        assert_eq!(value.value(at(start, 30)), 1.0);
    }
}

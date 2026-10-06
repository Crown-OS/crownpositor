//! Rectangles that glide from where they were to where the layout now puts
//! them.
//!
//! A relayout moves things in one step. Gliding them is FLIP: whatever moved
//! keeps drawing from where it *was*, and an offset spring carries it the rest
//! of the way. The layout stays the one source of truth about where things
//! are; the motion is purely how the change is shown.

use std::{collections::HashMap, hash::Hash};

use smithay::utils::{Logical, Rectangle};

use crate::animations::spring::{Spring, SpringProfile};

/// A rectangle's offset from its layout position — x, y, width, height —
/// springing back to zero.
#[derive(Debug, Clone, Copy)]
pub struct Glide {
    springs: [Spring; 4],
}

impl Glide {
    fn new(profile: SpringProfile) -> Self {
        Self {
            springs: [Spring::with_profile(0.0, profile); 4],
        }
    }

    /// Starts from `from` towards `to`, already moving at `velocity` logical
    /// pixels per second: a window let go of mid-flight keeps the speed the
    /// hand gave it.
    fn launch(
        &mut self,
        from: Rectangle<f64, Logical>,
        to: Rectangle<f64, Logical>,
        velocity: (f64, f64),
    ) {
        let offsets = [
            from.loc.x - to.loc.x,
            from.loc.y - to.loc.y,
            from.size.w - to.size.w,
            from.size.h - to.size.h,
        ];
        let speeds = [velocity.0, velocity.1, 0.0, 0.0];
        for ((spring, offset), speed) in self.springs.iter_mut().zip(offsets).zip(speeds) {
            spring.position = offset as f32;
            spring.velocity = speed as f32;
            spring.set_target(0.0);
        }
    }

    fn apply(&self, to: Rectangle<f64, Logical>) -> Rectangle<f64, Logical> {
        let [x, y, w, h] = self.springs.map(|spring| f64::from(spring.position));
        Rectangle::new(
            (to.loc.x + x, to.loc.y + y).into(),
            ((to.size.w + w).max(0.0), (to.size.h + h).max(0.0)).into(),
        )
    }

    fn step(&mut self, dt: f32) {
        for spring in &mut self.springs {
            spring.step(dt);
        }
    }

    fn at_rest(&self) -> bool {
        self.springs.iter().all(Spring::at_rest)
    }
}

/// Glides by identity, so a relayout that renumbers things does not hand one
/// thing's motion to another.
#[derive(Debug, Clone)]
pub struct Glides<K> {
    glides: HashMap<K, Glide>,
    /// `None` switches motion off: everything lands where the layout says.
    profile: Option<SpringProfile>,
}

impl<K: Eq + Hash + Copy> Glides<K> {
    pub fn new(profile: Option<SpringProfile>) -> Self {
        Self {
            glides: HashMap::new(),
            profile,
        }
    }

    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.profile = profile;
        if profile.is_none() {
            self.glides.clear();
        }
    }

    /// Shows `key` moving from `from` to `to` rather than appearing there.
    pub fn launch(
        &mut self,
        key: K,
        from: Rectangle<f64, Logical>,
        to: Rectangle<f64, Logical>,
        velocity: (f64, f64),
    ) {
        let Some(profile) = self.profile else {
            return;
        };
        if from == to && velocity == (0.0, 0.0) {
            self.glides.remove(&key);
            return;
        }
        self.glides
            .entry(key)
            .or_insert_with(|| Glide::new(profile))
            .launch(from, to, velocity);
    }

    /// Where `key` is drawn this frame, given where the layout puts it.
    pub fn rect(&self, key: K, to: Rectangle<f64, Logical>) -> Rectangle<f64, Logical> {
        self.glides.get(&key).map_or(to, |glide| glide.apply(to))
    }

    pub fn step(&mut self, dt: f32) {
        for glide in self.glides.values_mut() {
            glide.step(dt);
        }
        self.glides.retain(|_, glide| !glide.at_rest());
    }

    pub fn at_rest(&self) -> bool {
        self.glides.is_empty()
    }

    pub fn settle(&mut self) {
        self.glides.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_glide_starts_where_it_was_and_ends_where_it_is() {
        let mut glides = Glides::new(Some(SpringProfile::GESTURE));
        let (from, to) = (rect(0.0, 0.0, 100.0, 50.0), rect(300.0, 40.0, 200.0, 100.0));
        glides.launch(1, from, to, (0.0, 0.0));

        assert_eq!(glides.rect(1, to), from);
        for _ in 0..240 {
            glides.step(1.0 / 60.0);
        }
        assert!(glides.at_rest());
        assert_eq!(glides.rect(1, to), to);
    }

    #[test]
    fn a_thrown_glide_keeps_the_speed_it_was_given() {
        let mut glides = Glides::new(Some(SpringProfile::GESTURE));
        let to = rect(0.0, 0.0, 10.0, 10.0);
        glides.launch(7, to, to, (-800.0, 0.0));
        glides.step(1.0 / 60.0);
        assert!(glides.rect(7, to).loc.x < 0.0, "the throw carried it on");
    }

    #[test]
    fn without_motion_everything_lands_at_once() {
        let mut glides = Glides::new(None);
        let to = rect(10.0, 10.0, 10.0, 10.0);
        glides.launch(3, rect(0.0, 0.0, 1.0, 1.0), to, (0.0, 0.0));
        assert_eq!(glides.rect(3, to), to);
        assert!(glides.at_rest());
    }
}

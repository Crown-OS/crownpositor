//! One spring per hoverable thing, so a highlight grows and shrinks instead of
//! switching on.
//!
//! Every item keeps its own spring rather than one spring travelling between
//! them: moving the pointer from one thumbnail to the next has to shrink the
//! one being left at the same time as it grows the one being entered, and a
//! single value cannot be in two places.
//!
//! Springs are addressed by position, which is what the overview already
//! indexes its grid and its bar by. A relayout resizes the run; the springs
//! that survive keep whatever they were in the middle of.

use crate::animations::spring::{Spring, SpringProfile};

/// Lift amounts for a run of items, 0 (untouched) to 1 (fully hovered).
#[derive(Debug, Default, Clone)]
pub struct Lifts {
    springs: Vec<Spring>,
    /// `None` disables motion: a hover lands on the frame it happens.
    profile: Option<SpringProfile>,
}

impl Lifts {
    pub fn new(profile: Option<SpringProfile>) -> Self {
        Self {
            springs: Vec::new(),
            profile,
        }
    }

    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.profile = profile;
        match profile {
            Some(profile) => {
                for spring in &mut self.springs {
                    spring.set_profile(profile);
                }
            }
            None => self.settle(),
        }
    }

    /// Grows or shrinks the run to `len` items. New ones start unhovered.
    pub fn resize(&mut self, len: usize) {
        self.springs.resize_with(len, || {
            Spring::with_profile(0.0, self.profile.unwrap_or(SpringProfile::SMOOTH))
        });
    }

    /// Points every spring at rest and `hovered` at full lift.
    pub fn aim(&mut self, hovered: Option<usize>) {
        for (index, spring) in self.springs.iter_mut().enumerate() {
            spring.set_target(f32::from(u8::from(Some(index) == hovered)));
        }
        if self.profile.is_none() {
            self.settle();
        }
    }

    pub fn step(&mut self, dt: f32) {
        for spring in &mut self.springs {
            spring.step(dt);
        }
    }

    pub fn at_rest(&self) -> bool {
        self.springs.iter().all(Spring::at_rest)
    }

    pub fn settle(&mut self) {
        for spring in &mut self.springs {
            spring.snap_to_target();
        }
    }

    /// How far item `index` has been lifted. Anything the run does not cover is
    /// simply not hovered.
    pub fn at(&self, index: usize) -> f64 {
        self.springs
            .get(index)
            .map_or(0.0, |spring| f64::from(spring.position.max(0.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps at 60 Hz until the run stops needing frames, then lands it — the
    /// same two-step the render loop performs.
    fn settled(lifts: &mut Lifts) {
        for _ in 0..600 {
            if lifts.at_rest() {
                lifts.settle();
                return;
            }
            lifts.step(1.0 / 60.0);
        }
        panic!("the hover never came to rest");
    }

    fn lifts(len: usize) -> Lifts {
        let mut lifts = Lifts::new(Some(SpringProfile::SMOOTH));
        lifts.resize(len);
        lifts
    }

    #[test]
    fn nothing_is_lifted_to_begin_with() {
        let lifts = lifts(3);
        assert!(lifts.at_rest());
        assert!((0..3).all(|index| lifts.at(index) == 0.0));
    }

    #[test]
    fn an_index_nobody_announced_is_not_hovered() {
        assert_eq!(lifts(2).at(7), 0.0);
    }

    #[test]
    fn hovering_grows_the_one_under_the_pointer_and_no_other() {
        let mut lifts = lifts(3);
        lifts.aim(Some(1));
        assert!(!lifts.at_rest(), "a hover is worth a frame");

        settled(&mut lifts);
        assert_eq!(lifts.at(1), 1.0);
        assert_eq!(lifts.at(0), 0.0);
        assert_eq!(lifts.at(2), 0.0);
    }

    #[test]
    fn moving_between_two_grows_one_while_the_other_shrinks() {
        let mut lifts = lifts(2);
        lifts.aim(Some(0));
        settled(&mut lifts);

        lifts.aim(Some(1));
        lifts.step(1.0 / 60.0);
        assert!(lifts.at(0) < 1.0 && lifts.at(0) > 0.0, "{}", lifts.at(0));
        assert!(lifts.at(1) > 0.0 && lifts.at(1) < 1.0, "{}", lifts.at(1));
    }

    #[test]
    fn leaving_returns_everything_to_rest() {
        let mut lifts = lifts(2);
        lifts.aim(Some(0));
        settled(&mut lifts);
        lifts.aim(None);
        settled(&mut lifts);

        assert_eq!(lifts.at(0), 0.0);
    }

    #[test]
    fn a_relayout_keeps_the_springs_it_still_has() {
        let mut lifts = lifts(2);
        lifts.aim(Some(1));
        settled(&mut lifts);

        lifts.resize(4);
        assert_eq!(lifts.at(1), 1.0, "an existing hover was forgotten");
        assert_eq!(lifts.at(3), 0.0);
    }

    #[test]
    fn disabled_motion_lands_on_the_frame_it_happens() {
        let mut lifts = Lifts::new(None);
        lifts.resize(2);
        lifts.aim(Some(0));

        assert_eq!(lifts.at(0), 1.0);
        assert!(lifts.at_rest());
    }

    #[test]
    fn turning_motion_off_mid_flight_lands_the_hover() {
        let mut lifts = lifts(2);
        lifts.aim(Some(0));
        lifts.step(1.0 / 60.0);
        lifts.set_profile(None);

        assert_eq!(lifts.at(0), 1.0);
        assert!(lifts.at_rest());
    }
}

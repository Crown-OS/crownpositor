//! Resistance past the end of a range.
//!
//! A gesture that has run out of room should not stop dead — it should get
//! heavier. Both the workspace viewport and the overview want that, in their
//! own units, so the curve lives here rather than in either of them.

/// Diminishing returns past an edge.
#[derive(Debug, Clone, Copy)]
pub struct RubberBand {
    /// How far past an edge the value may travel, in the value's own units.
    pub limit: f64,
    /// Resistance at the edge. 1.0 would track the input exactly at first.
    pub strength: f64,
}

impl RubberBand {
    pub const fn new(limit: f64, strength: f64) -> Self {
        Self { limit, strength }
    }

    /// Squashes a position that has run outside `min..=max`, leaving one that
    /// is inside untouched.
    pub fn clamp(&self, position: f64, min: f64, max: f64) -> f64 {
        if position < min {
            min - self.give(min - position)
        } else if position > max {
            max + self.give(position - max)
        } else {
            position
        }
    }

    /// The first units of overshoot nearly track the input, and no amount of
    /// pulling gets past [`RubberBand::limit`].
    fn give(&self, overshoot: f64) -> f64 {
        self.limit * (1.0 - 1.0 / (overshoot * self.strength / self.limit + 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAND: RubberBand = RubberBand::new(0.35, 0.5);

    #[test]
    fn a_position_inside_the_range_is_untouched() {
        assert_eq!(BAND.clamp(1.5, 0.0, 3.0), 1.5);
        assert_eq!(BAND.clamp(0.0, 0.0, 3.0), 0.0);
        assert_eq!(BAND.clamp(3.0, 0.0, 3.0), 3.0);
    }

    #[test]
    fn the_edges_give_a_little_but_never_past_the_limit() {
        let under = BAND.clamp(-1.0, 0.0, 3.0);
        assert!(under < 0.0 && under > -BAND.limit, "{under}");

        let over = BAND.clamp(4.0, 0.0, 3.0);
        assert!(over > 3.0 && over < 3.0 + BAND.limit, "{over}");
    }

    #[test]
    fn pulling_ten_times_as_hard_barely_gets_further() {
        let hard = BAND.clamp(-10.0, 0.0, 3.0);
        assert!(hard > -BAND.limit, "{hard}");
    }

    #[test]
    fn resistance_grows_the_further_it_is_pulled() {
        // Twice the pull is less than twice the travel.
        let near = -BAND.clamp(-0.2, 0.0, 3.0);
        let far = -BAND.clamp(-0.4, 0.0, 3.0);
        assert!(far > near && far < near * 2.0, "{near} then {far}");
    }
}

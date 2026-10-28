//! Lock-screen transitions as data.
//!
//! A host never animates anything itself: it asks for the [`LayerStyle`] of a
//! layer at a point in a [`Transition`] and draws with it. Each [`Effect`]
//! contributes through [`Motion`], so a new kind of transition is a new
//! `Effect` variant and one arm here — every host picks it up unchanged.

mod easing;
mod style;

use crownos_config::schema::{Edge, Effect, Transition};

pub use easing::Curve;
pub use style::LayerStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Enter,
    Leave,
}

/// How one effect displaces a layer from rest.
pub trait Motion {
    /// `amount` is how far from rest the layer is, `1` fully displaced.
    fn displace(&self, amount: f32, direction: Direction, style: &mut LayerStyle);
}

impl Motion for Effect {
    fn displace(&self, amount: f32, direction: Direction, style: &mut LayerStyle) {
        match *self {
            Self::Fade => style.opacity *= 1.0 - amount,
            Self::Blur { radius } => style.blur += radius * amount,
            Self::Slide { from, distance } => {
                let [x, y] = toward(from);
                let sign = match direction {
                    Direction::Enter => 1.0,
                    Direction::Leave => -1.0,
                };
                style.offset[0] += x * distance * amount * sign;
                style.offset[1] += y * distance * amount * sign;
            }
            Self::Zoom { from } => {
                let target = match direction {
                    Direction::Enter => from,
                    Direction::Leave => 2.0 - from,
                };
                style.scale *= 1.0 + (target - 1.0) * amount;
            }
        }
    }
}

fn toward(edge: Edge) -> [f32; 2] {
    match edge {
        Edge::Top => [0.0, -1.0],
        Edge::Bottom => [0.0, 1.0],
        Edge::Left => [-1.0, 0.0],
        Edge::Right => [1.0, 0.0],
    }
}

/// The style of a layer `progress` (linear time, `0..=1`) of the way through
/// `transition`.
pub fn sample(transition: &Transition, progress: f32, direction: Direction) -> LayerStyle {
    let eased = transition.easing.ease(progress);
    let amount = match direction {
        Direction::Enter => 1.0 - eased,
        Direction::Leave => eased,
    };
    let mut style = LayerStyle::REST;
    if transition.is_instant() {
        if direction == Direction::Leave && progress > 0.0 {
            style.opacity = 0.0;
        }
        return style;
    }
    for effect in &transition.effects {
        effect.displace(amount, direction, &mut style);
    }
    if direction == Direction::Leave && progress >= 1.0 {
        style.opacity = 0.0;
    }
    style
}

#[cfg(test)]
mod tests {
    use crownos_config::schema::Easing;

    use super::*;

    fn linear(effects: Vec<Effect>) -> Transition {
        Transition {
            effects,
            duration_ms: 100,
            easing: Easing::Linear,
        }
    }

    #[test]
    fn an_entrance_ends_at_rest() {
        let transition = linear(vec![
            Effect::Fade,
            Effect::Blur { radius: 20.0 },
            Effect::Zoom { from: 0.9 },
        ]);
        assert_eq!(sample(&transition, 1.0, Direction::Enter), LayerStyle::REST);
        let start = sample(&transition, 0.0, Direction::Enter);
        assert_eq!(start.opacity, 0.0);
        assert_eq!(start.blur, 20.0);
        assert!((start.scale - 0.9).abs() < 1e-6);
    }

    #[test]
    fn a_slide_carries_on_through_when_leaving() {
        let transition = linear(vec![Effect::Slide {
            from: Edge::Bottom,
            distance: 40.0,
        }]);
        assert_eq!(
            sample(&transition, 0.0, Direction::Enter).offset,
            [0.0, 40.0]
        );
        assert_eq!(
            sample(&transition, 0.5, Direction::Leave).offset,
            [0.0, -20.0]
        );
    }

    #[test]
    fn a_zoom_mirrors_when_leaving() {
        let transition = linear(vec![Effect::Zoom { from: 0.9 }]);
        let leaving = sample(&transition, 0.5, Direction::Leave);
        assert!((leaving.scale - 1.05).abs() < 1e-6);
    }

    #[test]
    fn a_finished_exit_is_invisible_whatever_its_effects() {
        let transition = linear(vec![Effect::Blur { radius: 8.0 }]);
        assert_eq!(sample(&transition, 1.0, Direction::Leave).opacity, 0.0);
        assert_eq!(
            sample(&Transition::none(), 0.5, Direction::Leave).opacity,
            0.0
        );
        assert_eq!(
            sample(&Transition::none(), 0.0, Direction::Enter),
            LayerStyle::REST
        );
    }
}

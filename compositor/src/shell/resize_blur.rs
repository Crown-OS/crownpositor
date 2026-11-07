//! The frost a window's content wears while the compositor resizes it.
//!
//! A client redraws at a new size only after it has seen the configure, so for
//! the length of a snap or a relayout the screen would show its old buffer
//! stretched or cropped into the new rect. Blurring the content over the
//! change hides that: it frosts as the resize starts and clears once the
//! springs have arrived and the client has drawn at the size it was given.

use spacecontrol::animations::spring::{Spring, SpringProfile};

/// How long a client may take to draw at its new size before the frost
/// clears anyway: a client that refuses the size must not stay blurred.
const CLIENT_GRACE: f32 = 0.6;

/// Where the resize stands, as far as clearing the frost is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeProgress {
    /// The window's size is still animating.
    Moving,
    /// The window has arrived and the client has not drawn at its size yet.
    AwaitingClient,
    /// The window has arrived and the client has drawn at its size.
    Arrived,
}

#[derive(Debug, Clone, Copy)]
pub struct ResizeBlur {
    strength: Spring,
    /// Seconds since the frost started, while it is still being held.
    held: Option<f32>,
}

impl Default for ResizeBlur {
    fn default() -> Self {
        Self {
            strength: Spring::with_profile(0.0, SpringProfile::SNAPPY),
            held: None,
        }
    }
}

impl ResizeBlur {
    pub fn start(&mut self) {
        self.held = Some(0.0);
        self.strength.set_profile(SpringProfile::SNAPPY);
        self.strength.set_target(1.0);
    }

    pub fn step(&mut self, dt: f32, progress: ResizeProgress) {
        self.strength.step(dt);
        let Some(held) = &mut self.held else {
            return;
        };
        *held += dt;
        let clear = match progress {
            ResizeProgress::Moving => false,
            ResizeProgress::AwaitingClient => *held >= CLIENT_GRACE,
            ResizeProgress::Arrived => true,
        };
        if clear {
            self.held = None;
            self.strength.set_profile(SpringProfile::SMOOTH);
            self.strength.set_target(0.0);
        }
    }

    /// 0 is the window as it is, 1 fully frosted.
    pub fn strength(&self) -> f32 {
        self.strength.position.clamp(0.0, 1.0)
    }

    pub fn is_animating(&self) -> bool {
        self.held.is_some() || !self.strength.at_rest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    fn run(blur: &mut ResizeBlur, frames: usize, progress: ResizeProgress) {
        for _ in 0..frames {
            blur.step(FRAME, progress);
        }
    }

    #[test]
    fn it_frosts_while_the_window_is_moving() {
        let mut blur = ResizeBlur::default();
        blur.start();
        run(&mut blur, 60, ResizeProgress::Moving);
        assert!(blur.strength() > 0.99);
        assert!(blur.is_animating(), "held until the resize arrives");
    }

    #[test]
    fn it_clears_once_the_client_has_drawn_at_its_size() {
        let mut blur = ResizeBlur::default();
        blur.start();
        run(&mut blur, 20, ResizeProgress::Moving);
        run(&mut blur, 120, ResizeProgress::Arrived);
        assert_eq!(blur.strength(), 0.0);
        assert!(!blur.is_animating());
    }

    #[test]
    fn it_waits_for_a_slow_client_but_not_forever() {
        let mut blur = ResizeBlur::default();
        blur.start();
        run(&mut blur, 10, ResizeProgress::AwaitingClient);
        assert!(blur.strength() > 0.0, "still waiting on the client");

        run(&mut blur, 120, ResizeProgress::AwaitingClient);
        assert_eq!(
            blur.strength(),
            0.0,
            "a client refusing the size must not stay blurred"
        );
    }

    #[test]
    fn a_new_resize_mid_clear_frosts_again() {
        let mut blur = ResizeBlur::default();
        blur.start();
        run(&mut blur, 20, ResizeProgress::Arrived);
        let clearing = blur.strength();

        blur.start();
        run(&mut blur, 5, ResizeProgress::Moving);
        assert!(blur.strength() > clearing);
    }
}

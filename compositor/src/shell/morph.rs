//! A window turning into its application's icon and back.
//!
//! One spring drives the whole transformation: position, size, corner radius
//! and opacity are all read off the same progress, so they move as a single
//! physical object instead of four animations that happen to overlap. Turning
//! around mid-flight only retargets that spring, so an interrupted open or
//! minimize carries its velocity into the reverse motion with no seam.

use smithay::utils::{Logical, Point, Rectangle, Size};

use spacecontrol::animations::spring::{Spring, SpringProfile};

/// A 0.36 s response: 95 % of the way out in under 300 ms. Damped just short
/// of critical, which ends the motion sooner than a critically damped spring's
/// long tail would; the 0.15 % it carries past the window is clamped away.
const GROW: SpringProfile = spring(0.36, 0.92);
/// Going home is a little quicker than coming out.
const SHRINK: SpringProfile = spring(0.30, 0.92);
/// The part of the journey, nearest the icon, over which the window's content
/// fades into the icon it is becoming.
const FADE_SPAN: f32 = 0.3;
/// How far a window with no icon to go to shrinks in place while it fades.
const IN_PLACE_SCALE: f64 = 0.86;
/// Side of the patch a window minimizes into when its application shows no
/// icon: the middle of the screen's bottom edge, where a dock would be.
const DOCKLESS_SIDE: f64 = 64.0;

/// A spring by its period and damping ratio, the way designers specify one.
const fn spring(response_seconds: f32, damping_ratio: f32) -> SpringProfile {
    let angular_frequency = std::f32::consts::TAU / response_seconds;
    SpringProfile {
        stiffness: angular_frequency * angular_frequency,
        damping: 2.0 * damping_ratio * angular_frequency,
    }
}

/// The screen edge a dock sits along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Where a window comes from and returns to, output-local.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconOrigin {
    pub rect: Rectangle<f64, Logical>,
    /// Corner radius in logical pixels.
    pub radius: f64,
    /// The edge the icon's dock lies along; `None` for a launcher spread
    /// over the screen, or no icon at all.
    pub edge: Option<Edge>,
}

impl IconOrigin {
    /// The window's own rect, shrunk about its centre: where a window with no
    /// icon closes into.
    pub fn in_place(window: Rectangle<f64, Logical>, radius: f64) -> Self {
        let size = window.size.to_point().upscale(IN_PLACE_SCALE);
        let loc = window.loc + (window.size.to_point() - size).downscale(2.0);
        Self {
            rect: Rectangle::new(loc, Size::from((size.x, size.y))),
            radius: radius * IN_PLACE_SCALE,
            edge: None,
        }
    }

    /// The middle of `area`'s bottom edge: where a window with no icon
    /// minimizes into.
    pub fn bottom_of(area: Rectangle<i32, Logical>) -> Self {
        let area = area.to_f64();
        let loc = Point::from((
            area.loc.x + (area.size.w - DOCKLESS_SIDE) / 2.0,
            area.loc.y + area.size.h - DOCKLESS_SIDE,
        ));
        Self {
            rect: Rectangle::new(loc, Size::from((DOCKLESS_SIDE, DOCKLESS_SIDE))),
            radius: DOCKLESS_SIDE / 2.0,
            edge: Some(Edge::Bottom),
        }
    }
}

/// Which end of the transformation a morph is heading for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphEnd {
    Window,
    Icon,
}

#[derive(Debug, Clone, Copy)]
pub struct Morph {
    origin: IconOrigin,
    /// 0 is the icon, 1 is the window.
    progress: Spring,
}

impl Morph {
    pub fn new(origin: IconOrigin, from: MorphEnd) -> Self {
        let start = match from {
            MorphEnd::Window => 1.0,
            MorphEnd::Icon => 0.0,
        };
        Self {
            origin,
            progress: Spring::with_profile(start, GROW),
        }
    }

    /// A morph standing still on the window: the window drawn as it is.
    pub fn resting() -> Self {
        Self::new(
            IconOrigin::in_place(Rectangle::default(), 0.0),
            MorphEnd::Window,
        )
    }

    pub fn origin(&self) -> IconOrigin {
        self.origin
    }

    /// Heads for `end`, from wherever the morph is and at whatever speed it
    /// is going.
    pub fn head_for(&mut self, end: MorphEnd, origin: IconOrigin) {
        self.origin = origin;
        let (target, profile) = match end {
            MorphEnd::Window => (1.0, GROW),
            MorphEnd::Icon => (0.0, SHRINK),
        };
        self.progress.set_target(target);
        self.progress.set_profile(profile);
    }

    pub fn heading(&self) -> MorphEnd {
        if self.progress.target > 0.5 {
            MorphEnd::Window
        } else {
            MorphEnd::Icon
        }
    }

    /// Advances the spring; returns where the morph came to rest, if it did.
    pub fn step(&mut self, dt: f32) -> Option<MorphEnd> {
        self.progress.step(dt);
        self.progress.at_rest().then(|| self.heading())
    }

    /// How far from the icon the morph is: 0 on the icon, 1 on the window.
    pub fn progress(&self) -> f64 {
        f64::from(self.progress.position.clamp(0.0, 1.0))
    }

    /// The rect drawn this frame, between the icon and `window`.
    pub fn rect(&self, window: Rectangle<f64, Logical>) -> Rectangle<f64, Logical> {
        let t = self.progress();
        let icon = self.origin.rect;
        Rectangle::new(
            (
                lerp(icon.loc.x, window.loc.x, t),
                lerp(icon.loc.y, window.loc.y, t),
            )
                .into(),
            (
                lerp(icon.size.w, window.size.w, t).max(1.0),
                lerp(icon.size.h, window.size.h, t).max(1.0),
            )
                .into(),
        )
    }

    /// The corner radius this frame, between the icon's and `window`'s.
    pub fn radius(&self, window: f64) -> f64 {
        lerp(self.origin.radius, window, self.progress())
    }

    /// The content's opacity: whole for most of the way, fading into the
    /// icon over the last stretch.
    pub fn alpha(&self) -> f32 {
        let t = (self.progress() as f32 / FADE_SPAN).min(1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

fn lerp(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 120.0;

    fn origin() -> IconOrigin {
        IconOrigin {
            rect: Rectangle::new((100.0, 900.0).into(), (64.0, 64.0).into()),
            radius: 14.0,
            edge: Some(Edge::Bottom),
        }
    }

    fn window() -> Rectangle<f64, Logical> {
        Rectangle::new((200.0, 100.0).into(), (1200.0, 800.0).into())
    }

    /// Steps until rest, returning how long it took and where it stopped.
    fn run(morph: &mut Morph) -> (f32, MorphEnd) {
        let mut elapsed = 0.0;
        loop {
            elapsed += FRAME;
            if let Some(end) = morph.step(FRAME) {
                return (elapsed, end);
            }
            assert!(elapsed < 2.0, "the morph never settled");
        }
    }

    #[test]
    fn a_grow_starts_on_the_icon_and_ends_on_the_window() {
        let mut morph = Morph::new(origin(), MorphEnd::Icon);
        morph.head_for(MorphEnd::Window, origin());
        assert_eq!(morph.rect(window()), origin().rect);
        assert_eq!(morph.radius(10.0), 14.0);
        assert_eq!(morph.alpha(), 0.0);

        let (elapsed, end) = run(&mut morph);
        assert_eq!(end, MorphEnd::Window);
        assert!(elapsed < 0.5, "took {elapsed}s");
        assert_eq!(morph.rect(window()), window());
        assert_eq!(morph.radius(10.0), 10.0);
        assert_eq!(morph.alpha(), 1.0);
    }

    #[test]
    fn the_window_is_mostly_open_within_three_hundred_milliseconds() {
        let mut morph = Morph::new(origin(), MorphEnd::Icon);
        morph.head_for(MorphEnd::Window, origin());
        for _ in 0..36 {
            morph.step(FRAME);
        }
        assert!(morph.progress() > 0.93, "only {} open", morph.progress());
    }

    #[test]
    fn it_never_draws_past_the_window() {
        let mut morph = Morph::new(origin(), MorphEnd::Icon);
        morph.head_for(MorphEnd::Window, origin());
        for _ in 0..240 {
            morph.step(FRAME);
            assert!(morph.progress.position <= 1.002, "overshot visibly");
            let rect = morph.rect(window());
            assert!(rect.size.w <= window().size.w && rect.size.h <= window().size.h);
        }
    }

    #[test]
    fn reversing_mid_flight_keeps_position_and_speed() {
        let mut morph = Morph::new(origin(), MorphEnd::Icon);
        morph.head_for(MorphEnd::Window, origin());
        for _ in 0..12 {
            morph.step(FRAME);
        }
        let (position, velocity) = (morph.progress.position, morph.progress.velocity);
        morph.head_for(MorphEnd::Icon, origin());
        assert_eq!(morph.progress.position, position);
        assert_eq!(morph.progress.velocity, velocity);
        assert!(velocity > 0.0);

        let (_, end) = run(&mut morph);
        assert_eq!(end, MorphEnd::Icon);
        assert_eq!(morph.alpha(), 0.0);
    }

    #[test]
    fn in_place_shrinks_about_the_centre() {
        let shrunk = IconOrigin::in_place(window(), 10.0);
        let centre = |rect: Rectangle<f64, Logical>| rect.loc + rect.size.to_point().downscale(2.0);
        assert_eq!(centre(shrunk.rect), centre(window()));
        assert!(shrunk.rect.size.w < window().size.w);
    }
}

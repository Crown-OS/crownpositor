//! A window turning into its application's icon and back.
//!
//! Three springs drive the transformation: the edge farthest from the icon,
//! the shape's length along the way it travels, and its breadth across. A
//! [`Choreography`] sets how they move against one another: together, which
//! keeps the shape a rectangle and position, size, corner radius and opacity
//! one physical object, or each on its own beat, which bends it into the
//! liquid shape of [`LiquidShape`]. Turning around mid-flight only retargets
//! the springs, so an interrupted open or minimize carries its velocity into
//! the reverse motion with no seam.

mod channel;
mod choreography;
mod liquid;

use protocols::crownos_app_icons::{IconRef, PlacedIcon};
use smithay::utils::{Logical, Point, Rectangle, Size};

pub use choreography::Choreography;
pub use liquid::{LiquidShape, Progress};

use channel::Channel;

/// The part of the journey, nearest the icon, over which the window's content
/// fades into the icon it is becoming.
const FADE_SPAN: f32 = 0.3;
/// The part of the journey, nearest the icon, over which a carried icon
/// fades out of the shape growing from it.
const ICON_FADE_SPAN: f32 = 0.3;
/// The share of the slowest channel's journey, nearest the window, over which
/// a liquid morph's content clears up.
const REVEAL_SPAN: f32 = 0.2;
/// How far a window with no icon to go to shrinks in place while it fades.
const IN_PLACE_SCALE: f64 = 0.86;
/// Side of the patch a window minimizes into when its application shows no
/// icon: the middle of the screen's bottom edge, where a dock would be.
const DOCKLESS_SIDE: f64 = 64.0;

/// The screen edge a dock sits along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Where a window comes from and returns to, output-local.
#[derive(Debug, Clone, PartialEq)]
pub struct IconOrigin {
    pub rect: Rectangle<f64, Logical>,
    /// Corner radius in logical pixels.
    pub radius: f64,
    /// The edge the icon's dock lies along; `None` for a launcher spread
    /// over the screen, or no icon at all.
    pub edge: Option<Edge>,
    /// The launcher icon itself, which a liquid morph lifts out of its
    /// launcher and carries; `None` where there is no icon to carry.
    pub carrier: Option<PlacedIcon>,
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
            carrier: None,
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
            carrier: None,
        }
    }
}

/// Which end of the transformation a morph is heading for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MorphEnd {
    Window,
    Icon,
}

#[derive(Debug, Clone)]
pub struct Morph {
    origin: IconOrigin,
    /// 0 is the icon, 1 is the window.
    travel: Channel,
    length: Channel,
    breadth: Channel,
    choreography: Choreography,
    /// Seconds since the morph last set off or turned.
    elapsed: f32,
    /// Seconds it has rested on its icon, carrying it.
    lingered: f32,
}

impl Morph {
    pub fn new(origin: IconOrigin, from: MorphEnd) -> Self {
        let choreography = Choreography::UNIFORM;
        let [travel, length, breadth] = choreography.resting_on(from);
        Self {
            origin,
            travel,
            length,
            breadth,
            choreography,
            elapsed: 0.0,
            lingered: 0.0,
        }
    }

    /// A morph standing still on the window: the window drawn as it is.
    pub fn resting() -> Self {
        Self::new(
            IconOrigin::in_place(Rectangle::default(), 0.0),
            MorphEnd::Window,
        )
    }

    pub fn origin(&self) -> &IconOrigin {
        &self.origin
    }

    /// Heads for `end`, from wherever the morph is and at whatever speed it
    /// is going.
    pub fn head_for(&mut self, end: MorphEnd, origin: IconOrigin) {
        self.origin = origin;
        let target = match end {
            MorphEnd::Window => 1.0,
            MorphEnd::Icon => 0.0,
        };
        for channel in self.channels_mut() {
            channel.head_for(target);
        }
        self.elapsed = 0.0;
        self.lingered = 0.0;
    }

    pub fn heading(&self) -> MorphEnd {
        if self.travel.target() > 0.5 {
            MorphEnd::Window
        } else {
            MorphEnd::Icon
        }
    }

    /// Advances the springs as `choreography` has them move; returns where
    /// the morph came to rest, if it did.
    pub fn step(&mut self, dt: f32, choreography: &Choreography) -> Option<MorphEnd> {
        self.choreography = *choreography;
        self.elapsed += dt;
        let moves = *choreography.moves(self.heading());
        let elapsed = self.elapsed;
        self.travel.step(dt, elapsed, moves.travel);
        self.length.step(dt, elapsed, moves.length);
        self.breadth.step(dt, elapsed, moves.breadth);
        if !self.channels().iter().all(|channel| channel.at_rest()) {
            return None;
        }
        if self.carried().is_some() {
            self.lingered += dt;
            if self.lingered < choreography.linger {
                return None;
            }
        }
        Some(self.heading())
    }

    /// How far from the icon the morph is: 0 on the icon, 1 on the window.
    pub fn progress(&self) -> f64 {
        let sum: f32 = self
            .channels()
            .iter()
            .map(|channel| channel.position())
            .sum();
        f64::from((sum / 3.0).clamp(0.0, 1.0))
    }

    /// How far each channel has come, overshoot included.
    pub fn channel_progress(&self) -> Progress {
        Progress {
            travel: f64::from(self.travel.position()),
            length: f64::from(self.length.position()),
            breadth: f64::from(self.breadth.position()),
        }
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
        smoothstep(self.progress() as f32 / FADE_SPAN)
    }

    /// The liquid shape this frame, between the icon and `window`, whose
    /// corners are rounded by `window_radius`.
    pub fn shape(&self, window: Rectangle<f64, Logical>, window_radius: f64) -> LiquidShape {
        LiquidShape::between(
            &self.origin,
            window,
            window_radius,
            self.channel_progress(),
            self.choreography.taper,
        )
    }

    /// How much of the window's own content shows through the liquid shape:
    /// none while it is in flight, all of it once the slowest channel has
    /// nearly arrived.
    pub fn reveal(&self) -> f32 {
        let slowest = self
            .channels()
            .iter()
            .map(|channel| channel.position())
            .fold(f32::INFINITY, f32::min);
        smoothstep((slowest - (1.0 - REVEAL_SPAN)) / REVEAL_SPAN)
    }

    /// How much of the carried icon shows in the shape: all of it on the
    /// icon, fading out as the leading edge leaves.
    pub fn icon_alpha(&self) -> f32 {
        1.0 - smoothstep(self.travel.position() / ICON_FADE_SPAN)
    }

    /// The launcher icon this morph holds lifted out of its launcher: while
    /// any of it shows in the shape, until the morph lands on the icon.
    pub fn carried(&self) -> Option<&IconRef> {
        let carrier = self.origin.carrier.as_ref()?;
        let landed = self.lingered > 0.0;
        (self.choreography.linger > 0.0 && self.icon_alpha() > 0.0 && !landed)
            .then_some(&carrier.icon)
    }

    /// Whether the shape draws the carried icon this frame.
    pub fn shows_icon(&self) -> bool {
        self.origin.carrier.is_some() && self.choreography.linger > 0.0 && self.icon_alpha() > 0.0
    }

    /// How fast the shape's fastest edge moves, in logical pixels a second.
    pub fn speed(&self, window: Rectangle<f64, Logical>) -> f64 {
        let icon = self.origin.rect;
        let reach = (window.loc.x - icon.loc.x)
            .abs()
            .max((window.loc.y - icon.loc.y).abs())
            .max((window.size.w - icon.size.w).abs())
            .max((window.size.h - icon.size.h).abs());
        let fastest = self
            .channels()
            .iter()
            .map(|channel| channel.velocity().abs())
            .fold(0.0, f32::max);
        f64::from(fastest) * reach
    }

    fn channels(&self) -> [&Channel; 3] {
        [&self.travel, &self.length, &self.breadth]
    }

    fn channels_mut(&mut self) -> [&mut Channel; 3] {
        [&mut self.travel, &mut self.length, &mut self.breadth]
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
            carrier: None,
        }
    }

    fn window() -> Rectangle<f64, Logical> {
        Rectangle::new((200.0, 100.0).into(), (1200.0, 800.0).into())
    }

    /// Steps until rest, returning how long it took and where it stopped.
    fn run(morph: &mut Morph) -> (f32, MorphEnd) {
        run_as(morph, &Choreography::UNIFORM)
    }

    fn run_as(morph: &mut Morph, choreography: &Choreography) -> (f32, MorphEnd) {
        let mut elapsed = 0.0;
        loop {
            elapsed += FRAME;
            if let Some(end) = morph.step(FRAME, choreography) {
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
            morph.step(FRAME, &Choreography::UNIFORM);
        }
        assert!(morph.progress() > 0.93, "only {} open", morph.progress());
    }

    #[test]
    fn it_never_draws_past_the_window() {
        let mut morph = Morph::new(origin(), MorphEnd::Icon);
        morph.head_for(MorphEnd::Window, origin());
        for _ in 0..240 {
            morph.step(FRAME, &Choreography::UNIFORM);
            assert!(morph.travel.position() <= 1.002, "overshot visibly");
            let rect = morph.rect(window());
            assert!(rect.size.w <= window().size.w && rect.size.h <= window().size.h);
        }
    }

    #[test]
    fn reversing_mid_flight_keeps_position_and_speed() {
        let mut morph = Morph::new(origin(), MorphEnd::Icon);
        morph.head_for(MorphEnd::Window, origin());
        for _ in 0..12 {
            morph.step(FRAME, &Choreography::UNIFORM);
        }
        let (position, velocity) = (morph.travel.position(), morph.travel.velocity());
        morph.head_for(MorphEnd::Icon, origin());
        assert_eq!(morph.travel.position(), position);
        assert_eq!(morph.travel.velocity(), velocity);
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

    /// Grows a liquid morph for `seconds`.
    fn liquid_grown_for(seconds: f32) -> Morph {
        let mut morph = Morph::new(origin(), MorphEnd::Icon);
        morph.head_for(MorphEnd::Window, origin());
        for _ in 0..(seconds / FRAME).round() as usize {
            morph.step(FRAME, &Choreography::LIQUID);
        }
        morph
    }

    fn liquid_shrunk_for(seconds: f32) -> Morph {
        let mut morph = Morph::new(origin(), MorphEnd::Window);
        morph.head_for(MorphEnd::Icon, origin());
        for _ in 0..(seconds / FRAME).round() as usize {
            morph.step(FRAME, &Choreography::LIQUID);
        }
        morph
    }

    #[test]
    fn a_liquid_grow_shoots_its_leading_edge_past_the_window_and_back() {
        let peak = (1..=60)
            .map(|frame| liquid_grown_for(frame as f32 * FRAME).travel.position())
            .fold(0.0, f32::max);
        assert!((1.10..1.20).contains(&peak), "peaked at {peak}");
        let (elapsed, end) = run_as(&mut liquid_grown_for(0.0), &Choreography::LIQUID);
        assert_eq!(end, MorphEnd::Window);
        assert!(elapsed < 1.5, "took {elapsed}s");
    }

    #[test]
    fn a_liquid_grow_shows_no_content_until_it_has_nearly_arrived() {
        assert_eq!(liquid_grown_for(0.2).reveal(), 0.0);
        assert!(liquid_grown_for(0.3).channel_progress().travel > 0.9);
        assert!(liquid_grown_for(0.7).reveal() > 0.9);
    }

    #[test]
    fn a_liquid_shrink_drops_its_content_then_collapses_before_it_travels() {
        assert_eq!(liquid_shrunk_for(0.08).reveal(), 0.0);
        let collapsing = liquid_shrunk_for(0.12).channel_progress();
        assert_eq!(collapsing.travel, 1.0);
        assert!(collapsing.breadth < 0.8, "breadth {}", collapsing.breadth);
        let (_, end) = run_as(&mut liquid_shrunk_for(0.0), &Choreography::LIQUID);
        assert_eq!(end, MorphEnd::Icon);
    }

    #[test]
    fn turning_a_liquid_morph_mid_flight_never_freezes_it() {
        let mut morph = liquid_grown_for(0.15);
        let before = morph.channel_progress();
        morph.head_for(MorphEnd::Icon, origin());
        morph.step(FRAME, &Choreography::LIQUID);
        assert_ne!(morph.channel_progress(), before);
    }

    #[test]
    fn a_morph_without_an_icon_to_carry_carries_none_and_never_lingers() {
        let mut morph = liquid_shrunk_for(0.0);
        assert!(morph.carried().is_none());
        assert!(!morph.shows_icon());
        let (elapsed, _) = run_as(&mut morph, &Choreography::LIQUID);
        let mut uniform = liquid_shrunk_for(0.0);
        let mut settled = 0.0;
        while uniform.step(FRAME, &Choreography::LIQUID).is_none() {
            settled += FRAME;
        }
        assert!((elapsed - settled - FRAME).abs() < 1e-4);
    }
}

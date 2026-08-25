use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle, Serial, Size},
    wayland::shell::xdg::ToplevelSurface,
};

use config::ResolvedRule;

use crate::{
    animations::spring::{Spring, SpringProfile},
    layout::TileInfo,
    utils::id::WindowId,
};

/// What a window *is*, not what it looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    /// Positioned by the workspace's layout.
    Tiled,
    /// Keeps its own rect and is never passed to the layout.
    Floating,
    /// Fills the usable area, so panels stay visible.
    Maximized,
    /// Fills the whole output, ignoring exclusive zones.
    Fullscreen,
}

impl WindowState {
    pub fn is_tiled(self) -> bool {
        matches!(self, Self::Tiled)
    }

    /// Floating windows and the two overriding states all render above tiles.
    pub fn is_floating(self) -> bool {
        matches!(self, Self::Floating)
    }
}

pub struct Tile {
    id: WindowId,
    window: Window,
    /// Cached so removal can un-index without touching a possibly-dead object.
    surface: WlSurface,

    state: WindowState,
    /// Where to return on unmaximize/unfullscreen. Only ever `Tiled` or
    /// `Floating`, which is why it is not a stack.
    restore_state: WindowState,
    /// Kept current while floating, so Floating -> Tiled -> Floating is lossless.
    floating_rect: Rectangle<i32, Logical>,

    /// What the layout decided — the geometry authority.
    target: Rectangle<i32, Logical>,
    /// Size and serial of the last configure we sent.
    sent: Option<(Size<i32, Logical>, Serial)>,

    anim: TileAnim,
    /// Whether the layout has positioned this window yet. The first placement
    /// snaps — sliding in from the origin reads as a bug, not as polish.
    placed: bool,
    rules: ResolvedRule,
    /// Resolved once at map time from `appearance.transparency` and any window
    /// rule, so the render path does not consult the config per frame.
    opacity: f32,
    min_size: Size<i32, Logical>,
    max_size: Size<i32, Logical>,
}

impl Tile {
    pub fn new(
        id: WindowId,
        window: Window,
        surface: WlSurface,
        rules: ResolvedRule,
        opacity: f32,
    ) -> Self {
        let state = if rules.floating.unwrap_or(false) {
            WindowState::Floating
        } else {
            WindowState::Tiled
        };

        Self {
            id,
            window,
            surface,
            state,
            restore_state: state,
            floating_rect: Rectangle::default(),
            target: Rectangle::default(),
            sent: None,
            anim: TileAnim::new(Rectangle::default()),
            placed: false,
            rules,
            opacity,
            min_size: Size::default(),
            max_size: Size::default(),
        }
    }

    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity.clamp(0.0, 1.0);
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    pub fn toplevel(&self) -> Option<&ToplevelSurface> {
        self.window.toplevel()
    }

    pub fn state(&self) -> WindowState {
        self.state
    }

    pub fn rules(&self) -> &ResolvedRule {
        &self.rules
    }

    pub fn target(&self) -> Rectangle<i32, Logical> {
        self.target
    }

    pub fn floating_rect(&self) -> Rectangle<i32, Logical> {
        self.floating_rect
    }

    pub fn set_floating_rect(&mut self, rect: Rectangle<i32, Logical>) {
        self.floating_rect = rect;
    }

    pub fn info(&self) -> TileInfo {
        TileInfo {
            id: self.id,
            min_size: self.min_size,
            max_size: self.max_size,
        }
    }

    /// Refreshed from the client's size hints on commit.
    pub fn set_size_hints(&mut self, min: Size<i32, Logical>, max: Size<i32, Logical>) {
        self.min_size = min;
        self.max_size = max;
    }

    /// Returns whether the *size* changed, which is what gates a configure —
    /// moving a window does not require telling it anything.
    pub fn set_target(&mut self, rect: Rectangle<i32, Logical>) -> bool {
        let resized = self.target.size != rect.size;
        self.target = rect;
        if self.state.is_floating() {
            self.floating_rect = rect;
        }
        if self.placed {
            self.anim.retarget(rect);
        } else {
            self.placed = true;
            self.anim.snap(rect);
            self.anim.fade_in();
        }
        resized
    }

    /// Places a window without animating, for its first frame.
    pub fn snap_to(&mut self, rect: Rectangle<i32, Logical>) {
        self.target = rect;
        if self.state.is_floating() {
            self.floating_rect = rect;
        }
        self.anim.snap(rect);
    }

    pub fn anim(&self) -> &TileAnim {
        &self.anim
    }

    pub fn anim_mut(&mut self) -> &mut TileAnim {
        &mut self.anim
    }

    /// Where to draw this frame: the interpolated rect, not the target.
    pub fn render_rect(&self) -> Rectangle<f64, Logical> {
        self.anim.rect()
    }

    /// Opacity times the fade-in, so a new window appears rather than pops.
    pub fn render_alpha(&self) -> f32 {
        self.opacity * self.anim.alpha()
    }

    pub fn record_sent(&mut self, size: Size<i32, Logical>, serial: Serial) {
        self.sent = Some((size, serial));
    }

    /// Whether a configure would say anything new.
    pub fn needs_configure(&self) -> bool {
        self.sent.is_none_or(|(size, _)| size != self.target.size)
    }

    /// Moves between states, remembering where to come back to.
    ///
    /// Re-entering the state you are already in is a no-op rather than
    /// overwriting `restore_state` with itself, which would strand a window in
    /// fullscreen with nothing to restore.
    pub fn set_state(&mut self, state: WindowState) {
        if self.state == state {
            return;
        }
        if matches!(self.state, WindowState::Tiled | WindowState::Floating) {
            self.restore_state = self.state;
        }
        self.state = state;
    }

    pub fn restore(&mut self) {
        self.set_state(self.restore_state);
    }

    /// Toggles between tiled and floating, keeping the current rect as the
    /// floating one so a window does not jump when it pops out.
    pub fn toggle_floating(&mut self) {
        match self.state {
            WindowState::Tiled => {
                self.floating_rect = self.target;
                self.set_state(WindowState::Floating);
            }
            WindowState::Floating => self.set_state(WindowState::Tiled),
            // From an overriding state, un-override first.
            _ => self.restore(),
        }
    }

    pub fn alive(&self) -> bool {
        smithay::utils::IsAlive::alive(&self.window)
    }
}

/// Five springs: x, y, w, h, alpha.
///
/// These animate *rendering only*. `set_target` emits one configure at the final
/// size — sending each intermediate spring value would be ~60 configures per
/// second per moving window, and every client redrawing its whole surface for a
/// single resize.
#[derive(Debug, Clone, Copy)]
pub struct TileAnim {
    x: Spring,
    y: Spring,
    w: Spring,
    h: Spring,
    /// 0 -> 1 on map.
    alpha: Spring,
}

impl TileAnim {
    pub fn new(rect: Rectangle<i32, Logical>) -> Self {
        Self {
            // Position reads better unhurried; size has to keep up with the
            // content redrawing inside it, or a resize looks broken.
            x: Spring::with_profile(rect.loc.x as f32, SpringProfile::SMOOTH),
            y: Spring::with_profile(rect.loc.y as f32, SpringProfile::SMOOTH),
            w: Spring::with_profile(rect.size.w as f32, SpringProfile::SNAPPY),
            h: Spring::with_profile(rect.size.h as f32, SpringProfile::SNAPPY),
            alpha: Spring::with_profile(0.0, SpringProfile::SNAPPY),
        }
    }

    fn springs_mut(&mut self) -> [&mut Spring; 5] {
        [
            &mut self.x,
            &mut self.y,
            &mut self.w,
            &mut self.h,
            &mut self.alpha,
        ]
    }

    pub fn retarget(&mut self, rect: Rectangle<i32, Logical>) {
        self.x.set_target(rect.loc.x as f32);
        self.y.set_target(rect.loc.y as f32);
        self.w.set_target(rect.size.w as f32);
        self.h.set_target(rect.size.h as f32);
    }

    pub fn fade_in(&mut self) {
        self.alpha.set_target(1.0);
    }

    /// Jumps the geometry to `rect` without animating. Alpha is left alone, so a
    /// window can be placed instantly and still fade in.
    pub fn snap(&mut self, rect: Rectangle<i32, Logical>) {
        self.retarget(rect);
        for spring in [&mut self.x, &mut self.y, &mut self.w, &mut self.h] {
            spring.snap_to_target();
        }
    }

    pub fn step(&mut self, dt: f32) {
        for spring in self.springs_mut() {
            spring.step(dt);
        }
    }

    pub fn at_rest(&self) -> bool {
        self.x.at_rest()
            && self.y.at_rest()
            && self.w.at_rest()
            && self.h.at_rest()
            && self.alpha.at_rest()
    }

    /// Lands the final frame on exact integers, so nothing rests half a pixel off.
    pub fn settle(&mut self) {
        for spring in self.springs_mut() {
            spring.snap_to_target();
        }
    }

    pub fn rect(&self) -> Rectangle<f64, Logical> {
        Rectangle::new(
            (self.x.position as f64, self.y.position as f64).into(),
            (
                (self.w.position as f64).max(1.0),
                (self.h.position as f64).max(1.0),
            )
                .into(),
        )
    }

    pub fn alpha(&self) -> f32 {
        self.alpha.position.clamp(0.0, 1.0)
    }
}

impl std::fmt::Debug for Tile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tile")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Steps at 60 fps until everything settles, with a cap so a broken spring
    /// fails the test instead of hanging it.
    fn settle(anim: &mut TileAnim) -> usize {
        for frame in 0..600 {
            if anim.at_rest() {
                return frame;
            }
            anim.step(1.0 / 60.0);
        }
        panic!("the springs never came to rest");
    }

    #[test]
    fn snap_places_geometry_without_animating() {
        let mut anim = TileAnim::new(rect(0, 0, 0, 0));
        anim.snap(rect(100, 50, 800, 600));

        let placed = anim.rect();
        assert_eq!(placed.loc.x, 100.0);
        assert_eq!(placed.loc.y, 50.0);
        assert_eq!(placed.size.w, 800.0);
        assert_eq!(placed.size.h, 600.0);
    }

    #[test]
    fn snap_leaves_alpha_alone_so_a_window_can_still_fade_in() {
        let mut anim = TileAnim::new(rect(0, 0, 0, 0));
        anim.snap(rect(10, 10, 100, 100));
        assert_eq!(anim.alpha(), 0.0, "still invisible");

        anim.fade_in();
        settle(&mut anim);
        assert!((anim.alpha() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn retarget_moves_gradually_and_arrives() {
        let mut anim = TileAnim::new(rect(0, 0, 100, 100));
        anim.snap(rect(0, 0, 100, 100));
        anim.retarget(rect(500, 0, 100, 100));

        anim.step(1.0 / 60.0);
        let x = anim.rect().loc.x;
        assert!(x > 0.0 && x < 500.0, "should be mid-flight, was {x}");

        settle(&mut anim);
        assert!((anim.rect().loc.x - 500.0).abs() < 1.0);
    }

    #[test]
    fn settle_lands_on_exact_integers() {
        let mut anim = TileAnim::new(rect(0, 0, 100, 100));
        anim.retarget(rect(37, 91, 640, 480));
        settle(&mut anim);
        // The spring's epsilon stops it near, not on, the target.
        anim.settle();

        assert_eq!(
            anim.rect(),
            Rectangle::new((37.0, 91.0).into(), (640.0, 480.0).into())
        );
    }

    #[test]
    fn a_resting_animation_reports_at_rest() {
        let mut anim = TileAnim::new(rect(0, 0, 100, 100));
        anim.snap(rect(0, 0, 100, 100));
        anim.fade_in();
        settle(&mut anim);
        assert!(
            anim.at_rest(),
            "this is what gates scheduling the next frame"
        );
    }

    #[test]
    fn size_never_animates_through_zero() {
        // A zero-width texture is not something the renderer should ever be
        // handed, even for one frame mid-shrink.
        let mut anim = TileAnim::new(rect(0, 0, 800, 600));
        anim.snap(rect(0, 0, 800, 600));
        anim.retarget(rect(0, 0, 1, 1));

        for _ in 0..300 {
            anim.step(1.0 / 60.0);
            let size = anim.rect().size;
            assert!(size.w >= 1.0 && size.h >= 1.0, "got {size:?}");
        }
    }
}

use smithay::{
    desktop::Window,
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::Mode as KdeMode,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, Serial, Size},
    wayland::shell::xdg::ToplevelSurface,
};

use config::ResolvedRule;

use spacecontrol::animations::spring::{Spring, SpringProfile};

use crate::{
    layout::{SnapZone, TileInfo, placement},
    shell::{
        decoration::{DecorationCommit, DecorationIds, Insets},
        morph::{IconOrigin, Morph, MorphEnd},
    },
    utils::id::WindowId,
};

/// Who decides how big a window is.
///
/// A floating window opens at the size its client asked for and keeps it: the
/// configure carries no size at all, which is how xdg-shell spells "you
/// choose". The moment the compositor has to impose one — tiling, snapping,
/// maximizing, or the user dragging an edge — it latches over and stays there,
/// because a window the user has sized should not be resized behind their back
/// by its own client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeAuthority {
    Client,
    Compositor,
}

/// Who decides where a window is and how big it is, right now.
///
/// The layout, normally: a new rect goes to the springs and the window eases
/// into place. While the user is dragging or resizing it the pointer owns the
/// geometry instead and the springs are pinned to the cursor — a window that
/// eases *toward* the pointer trails behind it, which reads as the compositor
/// being slow rather than as polish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryAuthority {
    Layout,
    Pointer,
}

/// What a window *is*, not what it looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    /// Positioned by the workspace's tiling layout.
    Tiled,
    /// Keeps its own rect and is never passed to the layout.
    Floating,
    /// Dragged onto a screen edge. The zone owns the rect, and the window's
    /// `floating_rect` is left untouched — which is what lets dragging it back
    /// off restore the size it had before.
    Snapped(SnapZone),
    /// Fills the usable area, so panels stay visible.
    Maximized,
    /// Fills the whole output, ignoring exclusive zones.
    Fullscreen,
}

impl WindowState {
    pub fn is_tiled(self) -> bool {
        matches!(self, Self::Tiled)
    }

    /// Whether `floating_rect` is this window's geometry authority.
    pub fn is_floating(self) -> bool {
        matches!(self, Self::Floating)
    }

    /// Whether the window stacks above the tiled ones. A snapped window is
    /// still a floating window that happens to be parked on an edge.
    pub fn floats_above(self) -> bool {
        matches!(self, Self::Floating | Self::Snapped(_))
    }
}

/// The band a window is drawn and hit-tested in, back to front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackLayer {
    Tiled,
    Floating,
    Fullscreen,
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

    size_authority: SizeAuthority,
    geometry_authority: GeometryAuthority,
    decoration_ids: DecorationIds,
    decoration_commit: DecorationCommit,
    /// The window's title, cached so the render path does not lock the client's
    /// xdg state once per frame.
    title: String,
    /// Titlebar height in logical pixels, resolved from the config when the
    /// window maps so the render path never consults the config per frame. Zero
    /// turns decoration off entirely.
    titlebar_height: i32,
    chrome: Chrome,
    /// Floated by the compositor's own judgement rather than by a rule: a
    /// picker or an alert, which wants no titlebar of ours.
    dialog: bool,

    anim: TileAnim,
    /// Turning into its application's icon, or back out of it.
    morph: Option<Morph>,
    /// Out of the layout and off the screen until it is restored. Still drawn
    /// while its morph carries it into the icon.
    minimized: bool,
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

/// Who draws a window's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chrome {
    Compositor,
    /// The client negotiated client-side decorations and draws its own
    /// titlebar: one from us would be a second frame.
    Client,
}

impl Chrome {
    /// Who draws the frame, by what the client negotiated. `xdg-decoration`
    /// speaks first and the KDE protocol GTK 3 uses second. A client that used
    /// neither draws its own, which is what both protocols' absence means:
    /// GTK 4 and libadwaita bind neither and always draw a headerbar.
    pub fn requested(xdg: Option<Mode>, kde: Option<KdeMode>) -> Self {
        match (xdg, kde) {
            (Some(Mode::ServerSide), _) | (None, Some(KdeMode::Server)) => Self::Compositor,
            _ => Self::Client,
        }
    }
}

impl Tile {
    pub fn new(
        id: WindowId,
        window: Window,
        surface: WlSurface,
        rules: ResolvedRule,
        opacity: f32,
        titlebar_height: i32,
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
            size_authority: if state.is_floating() {
                SizeAuthority::Client
            } else {
                SizeAuthority::Compositor
            },
            geometry_authority: GeometryAuthority::Layout,
            decoration_ids: DecorationIds::default(),
            decoration_commit: DecorationCommit::default(),
            title: String::new(),
            titlebar_height,
            chrome: Chrome::Compositor,
            dialog: false,
            anim: TileAnim::new(Rectangle::default()),
            morph: None,
            minimized: false,
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

    /// Takes the knobs a rule may change on a mapped window. Placement ones —
    /// floating, workspace, output — stay as they were decided at map time.
    pub fn apply_presentation_rules(&mut self, rules: &ResolvedRule, opacity: f32) {
        self.rules.opacity = rules.opacity;
        self.rules.corner_radius = rules.corner_radius;
        self.rules.tearing = rules.tearing;
        self.rules.vrr = rules.vrr;
        self.set_opacity(opacity);
    }

    /// The whole visible window, decoration included. What the layout assigns
    /// and what the renderer draws into.
    pub fn target(&self) -> Rectangle<i32, Logical> {
        self.target
    }

    /// Whether this window wears a frame.
    ///
    /// Keyed on the window's own state, not the workspace's mode, so a dialog
    /// floated inside a tiling workspace is decorated too. A maximized window
    /// keeps whichever answer it will return to.
    pub fn is_decorated(&self) -> bool {
        self.titlebar_height > 0
            && !self.dialog
            && self.chrome == Chrome::Compositor
            && match self.state {
                WindowState::Tiled | WindowState::Fullscreen => false,
                WindowState::Floating | WindowState::Snapped(_) => true,
                WindowState::Maximized => self.restore_state.is_floating(),
            }
    }

    /// Whether this window lives among the floating ones. A maximized or
    /// fullscreen window belongs wherever it will return to, so maximizing a
    /// floating window keeps it in front rather than dropping it behind the
    /// tiled ones.
    pub fn floats(&self) -> bool {
        match self.state {
            WindowState::Floating | WindowState::Snapped(_) => true,
            WindowState::Tiled => false,
            WindowState::Maximized | WindowState::Fullscreen => self.restore_state.is_floating(),
        }
    }

    pub fn stack_layer(&self) -> StackLayer {
        if self.state == WindowState::Fullscreen {
            StackLayer::Fullscreen
        } else if self.floats() {
            StackLayer::Floating
        } else {
            StackLayer::Tiled
        }
    }

    /// Whether the compositor answers for this window's edges. Only a framed
    /// floating window: a client drawing its own chrome has its own resize
    /// handles, and every other state has its size decided for it.
    pub fn has_resize_band(&self) -> bool {
        self.state.is_floating() && self.is_decorated()
    }

    pub fn insets(&self) -> Insets {
        if self.is_decorated() {
            Insets::top(self.titlebar_height)
        } else {
            Insets::NONE
        }
    }

    pub fn chrome(&self) -> Chrome {
        self.chrome
    }

    pub fn set_chrome(&mut self, chrome: Chrome) {
        self.chrome = chrome;
    }

    pub fn set_dialog(&mut self, dialog: bool) {
        self.dialog = dialog;
    }

    pub fn set_titlebar_height(&mut self, height: i32) {
        self.titlebar_height = height.max(0);
    }

    pub fn decoration_ids(&self) -> &DecorationIds {
        &self.decoration_ids
    }

    /// The commit counter for a frame drawn with this appearance. Changes only
    /// when the appearance does, which is what stops the frame repainting every
    /// time the window moves.
    pub fn decoration_commit(
        &self,
        appearance: u64,
    ) -> smithay::backend::renderer::utils::CommitCounter {
        self.decoration_commit.get(appearance)
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns whether it changed, which is what gates re-rasterising it.
    pub fn set_title(&mut self, title: &str) -> bool {
        if self.title == title {
            return false;
        }
        self.title.clear();
        self.title.push_str(title);
        true
    }

    /// The part of the frame the client actually draws into.
    pub fn content_rect(&self) -> Rectangle<i32, Logical> {
        self.insets().content(self.target)
    }

    /// Where the client's own `(0, 0)` lands: its window geometry is what
    /// fills the content rect, and a client that draws shadows around itself
    /// puts that geometry some way into its buffer.
    pub fn surface_origin(&self) -> Point<i32, Logical> {
        self.content_rect().loc - self.window.geometry().loc
    }

    pub fn content_size(&self) -> Size<i32, Logical> {
        self.insets().content_size(self.target.size)
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

    pub fn size_authority(&self) -> SizeAuthority {
        self.size_authority
    }

    /// Takes the sizing decision away from the client, for good.
    pub fn claim_size(&mut self) {
        self.size_authority = SizeAuthority::Compositor;
    }

    /// Hands the window's geometry to a drag in progress.
    pub fn follow_pointer(&mut self) {
        self.geometry_authority = GeometryAuthority::Pointer;
    }

    /// Gives it back to the layout. Called before the drag's final rect is
    /// decided, so a window released onto an edge still glides into the snap
    /// rather than jumping there.
    pub fn release_pointer(&mut self) {
        self.geometry_authority = GeometryAuthority::Layout;
    }

    /// The size to put in a configure. `None` leaves the choice to the client,
    /// which is what a freshly floated window gets.
    pub fn configured_size(&self) -> Option<Size<i32, Logical>> {
        match self.size_authority {
            SizeAuthority::Client => None,
            SizeAuthority::Compositor => Some(self.content_size()),
        }
    }

    /// Follows a floating client that resized itself: one that still owns the
    /// decision, or one that took our size and committed another anyway.
    ///
    /// Returns whether anything moved, which is what the caller turns into a
    /// relayout — the new size still has to be clamped onto the screen.
    pub fn adopt_client_size(&mut self) -> bool {
        if !self.state.is_floating() {
            return false;
        }
        // The client speaks in content sizes; `floating_rect` is a frame.
        let requested = self.window.geometry().size;
        if requested.w <= 0 || requested.h <= 0 {
            return false;
        }
        if self.size_authority == SizeAuthority::Compositor && !self.refused(requested) {
            return false;
        }
        let frame = self.insets().frame_size(requested);
        if frame == self.floating_rect.size {
            return false;
        }
        self.floating_rect.size = frame;
        true
    }

    /// Returns whether the *size* changed, which is what gates a configure —
    /// moving a window does not require telling it anything.
    pub fn set_target(&mut self, rect: Rectangle<i32, Logical>) -> bool {
        let resized = self.target.size != rect.size;
        self.target = rect;
        if self.state.is_floating() {
            self.floating_rect = rect;
        }
        self.anim.set_titlebar(self.insets().top);
        if !self.placed {
            self.placed = true;
            self.anim.snap(rect);
            self.anim.fade_in();
        } else if self.geometry_authority == GeometryAuthority::Pointer {
            self.anim.follow(rect);
        } else {
            self.anim.retarget(rect);
        }
        resized
    }

    /// Places a window without animating, for its first frame.
    pub fn snap_to(&mut self, rect: Rectangle<i32, Logical>) {
        self.target = rect;
        if self.state.is_floating() {
            self.floating_rect = rect;
        }
        self.anim.set_titlebar(self.insets().top);
        self.anim.snap(rect);
    }

    pub fn anim(&self) -> &TileAnim {
        &self.anim
    }

    pub fn morph(&self) -> Option<&Morph> {
        self.morph.as_ref()
    }

    pub fn is_minimized(&self) -> bool {
        self.minimized
    }

    /// Whether the window is on screen: not minimized, or still on its way
    /// into the icon.
    pub fn is_shown(&self) -> bool {
        !self.minimized || self.morph.is_some()
    }

    /// Grows the window out of `origin`, or turns a shrink around.
    pub fn grow_from(&mut self, origin: IconOrigin) {
        self.anim.show();
        self.morph
            .get_or_insert_with(|| Morph::new(origin, MorphEnd::Icon))
            .head_for(MorphEnd::Window, origin);
    }

    /// Shrinks the window into `origin`, or turns a grow around.
    pub fn shrink_into(&mut self, origin: IconOrigin) {
        self.morph
            .get_or_insert_with(|| Morph::new(origin, MorphEnd::Window))
            .head_for(MorphEnd::Icon, origin);
    }

    pub fn minimize(&mut self, origin: IconOrigin) {
        self.minimized = true;
        self.shrink_into(origin);
    }

    /// Brings a minimized window back out of `origin`. It snaps to wherever
    /// the layout puts it next, rather than sliding there from where it was
    /// minimized.
    pub fn unminimize(&mut self, origin: IconOrigin) {
        self.minimized = false;
        self.placed = false;
        self.grow_from(origin);
    }

    /// Advances the morph; returns where it came to rest, if it did.
    pub fn step_morph(&mut self, dt: f32) -> Option<MorphEnd> {
        let end = self.morph.as_mut()?.step(dt)?;
        self.morph = None;
        Some(end)
    }

    pub fn anim_mut(&mut self) -> &mut TileAnim {
        &mut self.anim
    }

    /// Where to draw this frame: the interpolated rect, not the target.
    pub fn render_rect(&self) -> Rectangle<f64, Logical> {
        self.anim.rect()
    }

    /// The titlebar's height this frame, easing between shown and hidden.
    pub fn render_titlebar(&self) -> f64 {
        self.anim.titlebar()
    }

    /// How much of the titlebar is showing, from 0 (none) to 1 (all of it).
    pub fn titlebar_reveal(&self) -> f32 {
        (self.anim.titlebar() as f32 / self.titlebar_height.max(1) as f32).min(1.0)
    }

    /// Opacity times the fade-in, so a new window appears rather than pops.
    pub fn render_alpha(&self) -> f32 {
        self.opacity * self.anim.alpha()
    }

    /// Whether the client acked the size we last sent and committed another.
    /// A floating window's size is only ever a hint, and one below the
    /// client's minimum is not taken: clipping it to ours would cut it off.
    /// A client still catching up with a resize has not acked the latest size
    /// yet, so this does not fight a drag.
    fn refused(&self, committed: Size<i32, Logical>) -> bool {
        let Some((sent, _)) = self.sent else {
            return false;
        };
        let acked = self.window.toplevel().and_then(|toplevel| {
            toplevel.with_committed_state(|state| state.and_then(|state| state.size))
        });
        acked == Some(sent) && committed != sent
    }

    pub fn record_sent(&mut self, size: Size<i32, Logical>, serial: Serial) {
        self.sent = Some((size, serial));
    }

    /// Whether a configure would say anything new.
    pub fn needs_configure(&self) -> bool {
        self.sent
            .is_none_or(|(size, _)| size != self.content_size())
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

    /// Turns a tiled window loose at the size it should float at.
    ///
    /// A window that has floated before returns to the rect it left; one that
    /// never has takes the size its client asked for, which is what it would
    /// have opened at had the workspace been floating all along.
    pub fn float_into(&mut self, area: Rectangle<i32, Logical>, cascade: usize) {
        // The state has to move first: the insets the frame is built from
        // depend on it, and a tiled window reports none.
        self.set_state(WindowState::Floating);
        self.floating_rect = if self.floating_rect.is_empty() {
            placement::initial_rect(
                self.window.geometry().size,
                self.insets().top,
                area,
                None,
                cascade,
            )
        } else {
            placement::clamp_into(self.floating_rect, area)
        };
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

/// Six springs: x, y, w, h, alpha and the titlebar's height.
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
    titlebar: Spring,
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
            titlebar: Spring::with_profile(0.0, SpringProfile::SNAPPY),
        }
    }

    fn springs_mut(&mut self) -> [&mut Spring; 6] {
        [
            &mut self.x,
            &mut self.y,
            &mut self.w,
            &mut self.h,
            &mut self.alpha,
            &mut self.titlebar,
        ]
    }

    pub fn retarget(&mut self, rect: Rectangle<i32, Logical>) {
        self.x.set_target(rect.loc.x as f32);
        self.y.set_target(rect.loc.y as f32);
        self.w.set_target(rect.size.w as f32);
        self.h.set_target(rect.size.h as f32);
    }

    /// Puts the window exactly where the pointer has it.
    ///
    /// Every geometry spring is pinned rather than retargeted, so they exert no
    /// force and carry no velocity: the window is under the cursor on the very
    /// next frame instead of easing toward where it was. Size as well as
    /// position, because a resize drags an edge — an edge that eases toward the
    /// cursor is the same lag by another name.
    pub fn follow(&mut self, rect: Rectangle<i32, Logical>) {
        self.x.hold(rect.loc.x as f32);
        self.y.hold(rect.loc.y as f32);
        self.w.hold(rect.size.w as f32);
        self.h.hold(rect.size.h as f32);
    }

    /// Eases the titlebar towards `height`, so a frame gained or lost slides
    /// in or out instead of popping.
    pub fn set_titlebar(&mut self, height: i32) {
        self.titlebar.set_target(height as f32);
    }

    pub fn fade_in(&mut self) {
        self.alpha.set_target(1.0);
    }

    pub fn fade_out(&mut self) {
        self.alpha.set_target(0.0);
    }

    /// Fully opaque at once: a window growing out of an icon fades with its
    /// morph instead.
    pub fn show(&mut self) {
        self.alpha.set_target(1.0);
        self.alpha.snap_to_target();
    }

    /// Jumps the geometry to `rect` without animating. Alpha is left alone, so a
    /// window can be placed instantly and still fade in.
    pub fn snap(&mut self, rect: Rectangle<i32, Logical>) {
        self.retarget(rect);
        for spring in [
            &mut self.x,
            &mut self.y,
            &mut self.w,
            &mut self.h,
            &mut self.titlebar,
        ] {
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
            && self.titlebar.at_rest()
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

    pub fn titlebar(&self) -> f64 {
        (self.titlebar.position as f64).max(0.0)
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

    /// The whole point of `follow`: a dragged window is under the cursor on the
    /// very next frame, with nothing to step first.
    #[test]
    fn a_followed_position_lands_exactly_where_the_pointer_put_it() {
        let mut anim = TileAnim::new(rect(0, 0, 800, 600));
        anim.snap(rect(0, 0, 800, 600));

        anim.follow(rect(500, 300, 800, 600));

        let placed = anim.rect().loc;
        assert_eq!(placed.x, 500.0);
        assert_eq!(placed.y, 300.0);
    }

    /// Grabbing a window that was still flying must not let the speed it had
    /// built up carry it past the cursor.
    #[test]
    fn following_drops_the_velocity_the_spring_had_built_up() {
        let mut anim = TileAnim::new(rect(0, 0, 800, 600));
        anim.snap(rect(0, 0, 800, 600));
        anim.retarget(rect(900, 900, 800, 600));
        anim.step(1.0 / 60.0);
        assert!(anim.rect().loc.x > 0.0, "the spring should be moving");

        anim.follow(rect(100, 100, 800, 600));
        anim.step(1.0 / 60.0);

        assert_eq!(anim.rect().loc.x, 100.0);
        assert_eq!(anim.rect().loc.y, 100.0);
    }

    /// A resize drags an edge, so the size is pinned exactly like the position
    /// — the edge has to be under the cursor, not easing toward it.
    #[test]
    fn following_pins_the_size_as_well() {
        let mut anim = TileAnim::new(rect(0, 0, 800, 600));
        anim.snap(rect(0, 0, 800, 600));

        anim.follow(rect(0, 0, 400, 300));

        assert_eq!(anim.rect().size.w, 400.0);
        assert_eq!(anim.rect().size.h, 300.0);
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

    #[test]
    fn a_titlebar_slides_in_rather_than_popping() {
        let mut anim = TileAnim::new(rect(0, 0, 800, 600));
        anim.snap(rect(0, 0, 800, 600));
        anim.set_titlebar(36);

        anim.step(1.0 / 60.0);
        let height = anim.titlebar();
        assert!(height > 0.0 && height < 36.0, "got {height}");

        settle(&mut anim);
        anim.settle();
        assert_eq!(anim.titlebar(), 36.0);
    }

    #[test]
    fn a_placed_window_starts_with_its_titlebar_already_shown() {
        let mut anim = TileAnim::new(rect(0, 0, 0, 0));
        anim.set_titlebar(36);
        anim.snap(rect(0, 0, 800, 600));
        assert_eq!(anim.titlebar(), 36.0);
    }

    #[test]
    fn a_collapsing_titlebar_never_goes_negative() {
        let mut anim = TileAnim::new(rect(0, 0, 800, 600));
        anim.set_titlebar(36);
        anim.snap(rect(0, 0, 800, 600));
        anim.set_titlebar(0);

        for _ in 0..300 {
            anim.step(1.0 / 60.0);
            assert!(anim.titlebar() >= 0.0);
        }
    }

    #[test]
    fn a_client_that_negotiated_nothing_draws_its_own_frame() {
        assert_eq!(Chrome::requested(None, None), Chrome::Client);
    }

    #[test]
    fn xdg_decoration_wins_over_the_kde_protocol() {
        assert_eq!(
            Chrome::requested(Some(Mode::ClientSide), Some(KdeMode::Server)),
            Chrome::Client
        );
        assert_eq!(
            Chrome::requested(Some(Mode::ServerSide), Some(KdeMode::Client)),
            Chrome::Compositor
        );
    }

    #[test]
    fn the_kde_protocol_is_followed_without_xdg_decoration() {
        assert_eq!(
            Chrome::requested(None, Some(KdeMode::Server)),
            Chrome::Compositor
        );
        assert_eq!(
            Chrome::requested(None, Some(KdeMode::Client)),
            Chrome::Client
        );
        assert_eq!(Chrome::requested(None, Some(KdeMode::None)), Chrome::Client);
    }
}

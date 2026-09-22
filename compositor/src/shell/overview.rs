//! One output's overview: its state machine, and the layout it is showing.
//!
//! The `spacecontrol` crate does not know what a [`Tile`] is, so this is where
//! the shell model is turned into the flat snapshots it works in — and where
//! the indices it hands back become window ids again.
//!
//! Every workspace is laid out, not just the active one. A swipe across the
//! overview slides a neighbour's grid in, and it has to be somewhere already
//! for the slide to cost nothing but a different destination rectangle.
//!
//! The layout is cached rather than solved every frame. Nothing about it
//! changes while the overview animates: the windows are flying towards
//! rectangles that were fixed the moment it opened, so re-solving per frame
//! would be the same answer at the cost of running the solver sixty times a
//! second. [`Fingerprint`] is what notices when that stops being true.

use smithay::{
    backend::renderer::utils::CommitCounter,
    utils::{Logical, Point, Rectangle, Size},
};

use spacecontrol::{
    animations::{hover::Lifts, spring::SpringProfile},
    interaction::{Interaction, Release, Target},
    overview::Overview,
    render::{Chrome, Palette},
    scene::{self, Canvas, Metrics, Slot},
};

use crate::{shell::monitor::Monitor, utils::id::WindowId};

/// What the cached layout was computed from. When this changes the layout is
/// stale — a window opened, closed, resized, or the output did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    output: (i32, i32, i32, i32),
    usable: (i32, i32, i32, i32),
    workspaces: usize,
    active: usize,
    /// Every window's identity and size on every workspace, folded together.
    /// Order matters: a window raised above another re-reads the grid.
    windows: u64,
}

impl Fingerprint {
    fn of(monitor: &Monitor) -> Self {
        let geometry = monitor.geometry();
        let usable = monitor.usable();

        let windows = monitor
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.tiles())
            .fold(0xcbf2_9ce4_8422_2325, |hash, tile| {
                let size = tile.target().size;
                let mixed = tile.id().raw()
                    ^ (u64::from(size.w as u32) << 20)
                    ^ (u64::from(size.h as u32) << 40);
                (hash ^ mixed).wrapping_mul(0x1000_0000_01b3)
            });

        let corners =
            |rect: Rectangle<i32, Logical>| (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);

        Self {
            output: corners(geometry),
            usable: corners(usable),
            workspaces: monitor.workspaces().len(),
            active: monitor.active_index(),
            windows,
        }
    }
}

/// One workspace's thumbnails, all index-aligned.
#[derive(Debug, Default)]
struct Page {
    /// Where each window lands once the overview is fully open.
    grid: Vec<Rectangle<f64, Logical>>,
    windows: Vec<WindowId>,
    /// Where each window sits on the desktop, which is what a preview shrinks.
    desktop: Vec<Rectangle<i32, Logical>>,
}

/// The overview on one output.
#[derive(Debug)]
pub struct SpaceControl {
    overview: Overview,
    input: Interaction,
    chrome: Chrome,
    metrics: Metrics,
    palette: Palette,

    /// One per workspace, in the model's own order.
    pages: Vec<Page>,
    active: usize,
    bar: Vec<Slot>,
    /// Scratch for the solver, kept so a relayout allocates nothing.
    sizes: Vec<Size<i32, Logical>>,
    canvas: Canvas,
    fingerprint: Fingerprint,

    /// How far the pointer has lifted each window, and each workspace preview.
    windows_hover: Lifts,
    workspaces_hover: Lifts,
    profile: Option<SpringProfile>,

    /// Bumped whenever the blurred wallpaper's appearance changes. Without it
    /// the damage tracker sees an unmoved element with an unchanged commit and
    /// leaves the stale blur on screen for the whole animation.
    backdrop_commit: CommitCounter,
    blurred: f32,
}

impl Default for SpaceControl {
    fn default() -> Self {
        Self::new()
    }
}

impl SpaceControl {
    pub fn new() -> Self {
        let profile = Some(SpringProfile::SMOOTH);
        Self {
            overview: Overview::new(),
            input: Interaction::new(),
            chrome: Chrome::new(),
            metrics: Metrics::default(),
            palette: Palette::default(),
            pages: Vec::new(),
            active: 0,
            bar: Vec::new(),
            sizes: Vec::new(),
            canvas: Canvas::whole(Rectangle::default()),
            fingerprint: Fingerprint::default(),
            windows_hover: Lifts::new(profile),
            workspaces_hover: Lifts::new(profile),
            profile,
            backdrop_commit: CommitCounter::default(),
            blurred: 0.0,
        }
    }

    pub fn overview(&self) -> &Overview {
        &self.overview
    }

    pub fn chrome(&self) -> &Chrome {
        &self.chrome
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The output the overview laid itself out on, and the part of it nothing
    /// has reserved.
    pub fn canvas(&self) -> Canvas {
        self.canvas
    }

    /// The active workspace's thumbnail rectangles, which is what the pointer
    /// hit-tests against.
    pub fn grid(&self) -> &[Rectangle<f64, Logical>] {
        self.pages.get(self.active).map_or(&[], |page| &page.grid)
    }

    /// Where workspace `workspace`'s window `index` lands in the grid.
    pub fn page_grid(&self, workspace: usize) -> &[Rectangle<f64, Logical>] {
        self.pages.get(workspace).map_or(&[], |page| &page.grid)
    }

    pub fn bar(&self) -> &[Slot] {
        &self.bar
    }

    /// How far the pointer has lifted window `index` of the active workspace.
    /// Only the active workspace's grid is hoverable, so every other page's
    /// windows sit flat.
    pub fn window_lift(&self, workspace: usize, index: usize) -> f64 {
        if workspace != self.active {
            return 0.0;
        }
        self.windows_hover.at(index)
    }

    pub fn workspace_lift(&self, index: usize) -> f64 {
        self.workspaces_hover.at(index)
    }

    /// The window being carried and where to draw it, as an index into the
    /// active workspace's grid.
    ///
    /// Over a workspace preview the window goes *into* it rather than hovering
    /// above it, so a drop is seen to land rather than merely aimed at.
    pub fn carrying(&self) -> Option<(usize, Rectangle<f64, Logical>)> {
        let carry = self.input.carrying()?;
        let riding = carry.rect();

        let Some(Target::Workspace(workspace)) = self.input.hovered() else {
            return Some((carry.window, riding));
        };
        let Some(slot) = self.bar.get(workspace) else {
            return Some((carry.window, riding));
        };
        let Some(desktop) = self
            .pages
            .get(self.active)
            .and_then(|page| page.desktop.get(carry.window))
        else {
            return Some((carry.window, riding));
        };

        let landing = scene::inside(*desktop, self.canvas.output, self.thumb(*slot));
        Some((
            carry.window,
            scene::between(riding, landing, self.input.snap()),
        ))
    }

    /// One preview's box where it currently is, the bar's climb included.
    fn thumb(&self, slot: Slot) -> Rectangle<f64, Logical> {
        let climb = scene::climb(self.canvas, &self.metrics, self.overview.bar());
        Rectangle::new(
            (slot.thumb.loc.x, slot.thumb.loc.y + climb).into(),
            slot.thumb.size,
        )
    }

    /// The window a grid index refers to, on the workspace the pointer is
    /// working in.
    pub fn window(&self, index: usize) -> Option<WindowId> {
        self.pages
            .get(self.active)
            .and_then(|page| page.windows.get(index))
            .copied()
    }

    /// Recomputes the grid and the bar if anything they depend on moved.
    ///
    /// Called once per frame while the overview is on screen; the fingerprint
    /// makes all but the first of those free.
    pub fn relayout(&mut self, monitor: &Monitor) {
        let fingerprint = Fingerprint::of(monitor);
        if fingerprint == self.fingerprint && !self.pages.is_empty() {
            return;
        }
        self.fingerprint = fingerprint;
        self.resolve(monitor);
    }

    /// Recomputes unconditionally — for when the model changed under a
    /// fingerprint that happens to match, such as a window moving workspace.
    pub fn resolve(&mut self, monitor: &Monitor) {
        self.canvas = Canvas::new(monitor.geometry(), monitor.usable());
        self.active = monitor.active_index();

        self.pages
            .resize_with(monitor.workspaces().len(), Page::default);

        for (workspace, page) in monitor.workspaces().iter().zip(&mut self.pages) {
            page.windows.clear();
            page.desktop.clear();
            self.sizes.clear();
            for tile in workspace.tiles() {
                page.windows.push(tile.id());
                page.desktop.push(tile.target());
                self.sizes.push(tile.target().size);
            }
            scene::grid(self.canvas, &self.sizes, &self.metrics, &mut page.grid);
        }

        scene::bar(
            self.canvas,
            monitor.workspaces().len(),
            &self.metrics,
            &mut self.bar,
        );
        self.chrome.ensure(self.bar.len());
        self.windows_hover.resize(self.grid().len());
        self.workspaces_hover.resize(self.bar.len());
    }

    /// Opens the overview, laying it out first so the windows have somewhere to
    /// fly to on the very first frame.
    pub fn open(&mut self, monitor: &Monitor) {
        self.resolve(monitor);
        self.overview.open();
    }

    pub fn close(&mut self) {
        self.input.clear();
        self.aim_hover();
        self.overview.close();
    }

    pub fn toggle(&mut self, monitor: &Monitor) {
        if self.overview.is_open() {
            self.close();
        } else {
            self.open(monitor);
        }
    }

    pub fn is_open(&self) -> bool {
        self.overview.is_open()
    }

    pub fn is_visible(&self) -> bool {
        self.overview.is_visible()
    }

    pub fn is_active(&self) -> bool {
        self.overview.is_active()
            || !self.windows_hover.at_rest()
            || !self.workspaces_hover.at_rest()
            || !self.input.at_rest()
    }

    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.profile = profile;
        self.overview.set_profile(profile);
        self.input.set_profile(profile);
        self.windows_hover.set_profile(profile);
        self.workspaces_hover.set_profile(profile);
    }

    pub fn step(&mut self, dt: f32) {
        self.overview.step(dt);
        self.windows_hover.step(dt);
        self.workspaces_hover.step(dt);
        self.input.step(dt);

        let blur = self.overview.blur();
        if (blur - self.blurred).abs() > 1e-3 {
            self.blurred = blur;
            self.backdrop_commit.increment();
        }
    }

    pub fn backdrop_commit(&self) -> CommitCounter {
        self.backdrop_commit
    }

    pub fn settle(&mut self) {
        self.overview.settle();
        self.windows_hover.settle();
        self.workspaces_hover.settle();
        self.input.settle();
    }

    pub fn begin_gesture(&mut self, monitor: &Monitor) {
        if !self.overview.is_dragging() {
            self.resolve(monitor);
            self.overview.begin_gesture();
        }
    }

    pub fn update_gesture(&mut self, travelled: f64) {
        self.overview.update_gesture(travelled);
    }

    pub fn end_gesture(&mut self, velocity: f64) -> bool {
        let open = self.overview.end_gesture(velocity);
        if !open {
            self.input.clear();
            self.aim_hover();
        }
        open
    }

    pub fn cancel_gesture(&mut self) {
        self.overview.cancel_gesture();
    }

    pub fn is_dragging(&self) -> bool {
        self.overview.is_dragging()
    }

    /// Moves the pointer over the overview. Returns whether anything needs
    /// redrawing.
    pub fn motion(&mut self, at: Point<f64, Logical>) -> bool {
        // `Interaction` is a plain value, so this is the cheapest way to hand
        // the hit-test a shared borrow of the layout it is testing against.
        let mut input = self.input;
        let changed = input.motion(at, self.grid(), &self.bar);
        self.input = input;

        if changed {
            self.aim_hover();
        }
        changed
    }

    pub fn press(&mut self, at: Point<f64, Logical>) -> Option<Target> {
        let mut input = self.input;
        let target = input.press(at, self.grid(), &self.bar);
        self.input = input;
        self.aim_hover();
        target
    }

    pub fn release(&mut self, at: Point<f64, Logical>) -> Release {
        let release = self.input.release(at, &self.bar);
        self.aim_hover();
        release
    }

    /// Points the hover springs at whatever the pointer is over now.
    fn aim_hover(&mut self) {
        let hovered = self.input.hovered();
        self.windows_hover.aim(match hovered {
            Some(Target::Window(index)) => Some(index),
            _ => None,
        });
        self.workspaces_hover.aim(match hovered {
            Some(Target::Workspace(index)) => Some(index),
            _ => None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laid_out(windows: usize, workspaces: usize) -> SpaceControl {
        let mut space = SpaceControl::new();
        space.canvas = Canvas::whole(Rectangle::new((0, 0).into(), (1920, 1080).into()));
        space.pages = (0..workspaces)
            .map(|_| Page {
                grid: (0..windows)
                    .map(|index| {
                        Rectangle::new(
                            (f64::from(index as i32) * 300.0, 100.0).into(),
                            (200.0, 150.0).into(),
                        )
                    })
                    .collect(),
                windows: (0..windows).map(|_| WindowId::next()).collect(),
                desktop: (0..windows)
                    .map(|_| Rectangle::new((0, 0).into(), (800, 600).into()))
                    .collect(),
            })
            .collect();
        scene::bar(space.canvas, workspaces, &space.metrics, &mut space.bar);
        space.chrome.ensure(workspaces);
        space.windows_hover.resize(windows);
        space.workspaces_hover.resize(workspaces);
        space
    }

    #[test]
    fn a_fresh_overview_is_closed_and_empty() {
        let space = SpaceControl::new();
        assert!(!space.is_open());
        assert!(!space.is_visible());
        assert!(space.grid().is_empty());
        assert!(space.bar().is_empty());
        assert_eq!(space.window(0), None);
    }

    #[test]
    fn an_unknown_grid_index_names_no_window() {
        let space = SpaceControl::new();
        assert_eq!(space.window(7), None);
    }

    #[test]
    fn closing_forgets_an_unreleased_press() {
        let mut space = laid_out(1, 2);

        space.press((50.0, 150.0).into());
        space.motion((400.0, 400.0).into());
        assert!(space.carrying().is_some());

        space.close();
        assert!(space.carrying().is_none());
    }

    #[test]
    fn hovering_a_window_lifts_only_that_one() {
        let mut space = laid_out(3, 2);
        assert!(space.motion((350.0, 150.0).into()), "the hover is a change");

        space.settle();
        assert_eq!(space.window_lift(0, 1), 1.0);
        assert_eq!(space.window_lift(0, 0), 0.0);
    }

    #[test]
    fn a_window_on_another_workspace_never_lifts() {
        let mut space = laid_out(3, 2);
        space.motion((350.0, 150.0).into());
        space.settle();

        assert_eq!(space.window_lift(1, 1), 0.0, "an off-page window lifted");
    }

    #[test]
    fn a_hover_is_worth_animating() {
        let mut space = laid_out(2, 2);
        space.motion((50.0, 150.0).into());
        assert!(space.is_active(), "a hover has to be stepped to be seen");
    }

    #[test]
    fn hovering_a_workspace_lifts_its_preview() {
        let mut space = laid_out(1, 3);
        let slot = space.bar()[1];
        let at = slot.hit();

        space.motion((at.loc.x + at.size.w / 2.0, at.loc.y + at.size.h / 2.0).into());
        space.settle();
        assert_eq!(space.workspace_lift(1), 1.0);
        assert_eq!(space.workspace_lift(0), 0.0);
    }

    #[test]
    fn a_carried_window_settles_into_the_preview_it_is_over() {
        let mut space = laid_out(1, 3);
        space.overview.snap_to(true);
        space.press((50.0, 150.0).into());

        let slot = space.bar()[2];
        let at = Point::from((
            slot.thumb.loc.x + slot.thumb.size.w / 2.0,
            slot.thumb.loc.y + slot.thumb.size.h / 2.0,
        ));
        space.motion(at);

        let riding = space.carrying().expect("a carried window").1;
        for _ in 0..120 {
            space.step(1.0 / 60.0);
        }
        let landed = space.carrying().expect("a carried window").1;

        assert!(
            landed.size.w < riding.size.w,
            "{landed:?} never shrank into {slot:?}"
        );
        assert!(
            slot.thumb.to_f64().contains_rect(landed),
            "{landed:?} is not inside {:?}",
            slot.thumb
        );
    }

    #[test]
    fn a_fingerprint_notices_a_resized_window() {
        let a = Fingerprint {
            output: (0, 0, 1920, 1080),
            usable: (0, 32, 1920, 1048),
            workspaces: 2,
            active: 0,
            windows: 11,
        };
        assert_eq!(a, a);
        assert_ne!(a, Fingerprint { windows: 12, ..a });
        assert_ne!(a, Fingerprint { workspaces: 3, ..a });
        assert_ne!(a, Fingerprint { active: 1, ..a });
        assert_ne!(
            a,
            Fingerprint {
                usable: (0, 0, 1920, 1080),
                ..a
            }
        );
        assert_ne!(
            a,
            Fingerprint {
                output: (0, 0, 2560, 1080),
                ..a
            }
        );
    }
}

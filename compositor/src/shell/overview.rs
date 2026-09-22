//! One output's overview: its state machine, and the layout it is showing.
//!
//! The `spacecontrol` crate does not know what a [`Tile`] is, so this is where
//! the shell model is turned into the flat snapshots it works in — and where
//! the indices it hands back become window ids again.
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
    interaction::{Carry, Interaction, Release, Target},
    overview::Overview,
    render::{Chrome, Palette},
    scene::{self, Metrics, Slot},
};

use crate::{shell::monitor::Monitor, utils::id::WindowId};

/// What the cached layout was computed from. When this changes the layout is
/// stale — a window opened, closed, resized, or the output did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    output: (i32, i32, i32, i32),
    workspaces: usize,
    /// Every window's identity and size, folded together. Order matters: a
    /// window raised above another re-reads the grid.
    windows: u64,
}

impl Fingerprint {
    fn of(monitor: &Monitor) -> Self {
        let geometry = monitor.geometry();
        let windows = monitor
            .active()
            .tiles()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325, |hash, tile| {
                let size = tile.target().size;
                let mixed = tile.id().raw()
                    ^ (u64::from(size.w as u32) << 20)
                    ^ (u64::from(size.h as u32) << 40);
                (hash ^ mixed).wrapping_mul(0x1000_0000_01b3)
            });

        Self {
            output: (
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            ),
            workspaces: monitor.workspaces().len(),
            windows,
        }
    }
}

/// The overview on one output.
#[derive(Debug)]
pub struct SpaceControl {
    overview: Overview,
    input: Interaction,
    chrome: Chrome,
    metrics: Metrics,
    palette: Palette,

    /// Where each window of the active workspace lands, index-aligned with
    /// [`SpaceControl::windows`].
    grid: Vec<Rectangle<f64, Logical>>,
    windows: Vec<WindowId>,
    bar: Vec<Slot>,
    /// Scratch for the solver, kept so a relayout allocates nothing.
    sizes: Vec<Size<i32, Logical>>,
    fingerprint: Fingerprint,

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
        Self {
            overview: Overview::new(),
            input: Interaction::new(),
            chrome: Chrome::new(),
            metrics: Metrics::default(),
            palette: Palette::default(),
            grid: Vec::new(),
            windows: Vec::new(),
            bar: Vec::new(),
            sizes: Vec::new(),
            fingerprint: Fingerprint::default(),
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

    pub fn grid(&self) -> &[Rectangle<f64, Logical>] {
        &self.grid
    }

    pub fn bar(&self) -> &[Slot] {
        &self.bar
    }

    pub fn hovered(&self) -> Option<Target> {
        self.input.hovered()
    }

    /// The window being carried and where to draw it, as an index into the
    /// grid.
    pub fn carrying(&self) -> Option<(usize, Rectangle<f64, Logical>)> {
        self.input
            .carrying()
            .map(|carry: Carry| (carry.window, carry.rect()))
    }

    /// The window a grid index refers to.
    pub fn window(&self, index: usize) -> Option<WindowId> {
        self.windows.get(index).copied()
    }

    /// Recomputes the grid and the bar if anything they depend on moved.
    ///
    /// Called once per frame while the overview is on screen; the fingerprint
    /// makes all but the first of those free.
    pub fn relayout(&mut self, monitor: &Monitor) {
        let fingerprint = Fingerprint::of(monitor);
        if fingerprint == self.fingerprint && !self.grid.is_empty() {
            return;
        }
        self.fingerprint = fingerprint;
        self.resolve(monitor);
    }

    /// Recomputes unconditionally — for when the model changed under a
    /// fingerprint that happens to match, such as a window moving workspace.
    pub fn resolve(&mut self, monitor: &Monitor) {
        let geometry = monitor.geometry();

        self.windows.clear();
        self.sizes.clear();
        for tile in monitor.active().tiles() {
            self.windows.push(tile.id());
            self.sizes.push(tile.target().size);
        }

        scene::grid(geometry, &self.sizes, &self.metrics, &mut self.grid);
        scene::bar(
            geometry,
            monitor.workspaces().len(),
            &self.metrics,
            &mut self.bar,
        );
        self.chrome.ensure(self.bar.len());
    }

    /// Opens the overview, laying it out first so the windows have somewhere to
    /// fly to on the very first frame.
    pub fn open(&mut self, monitor: &Monitor) {
        self.resolve(monitor);
        self.overview.open();
    }

    pub fn close(&mut self) {
        self.input.clear();
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
    }

    pub fn step(&mut self, dt: f32) {
        self.overview.step(dt);

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
        self.input.motion(at, &self.grid, &self.bar)
    }

    pub fn press(&mut self, at: Point<f64, Logical>) -> Option<Target> {
        self.input.press(at, &self.grid, &self.bar)
    }

    pub fn release(&mut self, at: Point<f64, Logical>) -> Release {
        self.input.release(at, &self.bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut space = SpaceControl::new();
        space.grid = vec![Rectangle::new((0.0, 0.0).into(), (100.0, 100.0).into())];
        space.windows = vec![WindowId::next()];

        space.press((50.0, 50.0).into());
        space.motion((400.0, 400.0).into());
        assert!(space.carrying().is_some());

        space.close();
        assert!(space.carrying().is_none());
    }

    #[test]
    fn a_fingerprint_notices_a_resized_window() {
        let a = Fingerprint {
            output: (0, 0, 1920, 1080),
            workspaces: 2,
            windows: 11,
        };
        assert_eq!(a, a);
        assert_ne!(a, Fingerprint { windows: 12, ..a });
        assert_ne!(a, Fingerprint { workspaces: 3, ..a });
        assert_ne!(
            a,
            Fingerprint {
                output: (0, 0, 2560, 1080),
                ..a
            }
        );
    }
}

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
    animations::{glide::Glides, hover::Lifts, spring::SpringProfile},
    interaction::{BarView, Interaction, Release, Target},
    overview::Overview,
    render::{Chrome, Palette},
    scene::{self, Canvas, Metrics, Slot},
};

use crate::{
    shell::monitor::Monitor,
    utils::id::{WindowId, WorkspaceId},
};

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
    workspace: Option<WorkspaceId>,
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
    /// The '+' tile after the last preview.
    add: Option<Slot>,
    /// Scratch for the solver, kept so a relayout allocates nothing.
    sizes: Vec<Size<i32, Logical>>,
    canvas: Canvas,
    fingerprint: Fingerprint,

    /// How far the pointer has lifted each window, and each workspace preview
    /// — the '+' tile is the one after the last.
    windows_hover: Lifts,
    workspaces_hover: Lifts,
    profile: Option<SpringProfile>,

    /// Windows and previews on their way from where they were drawn to where a
    /// relayout put them.
    window_glides: Glides<WindowId>,
    preview_glides: Glides<WorkspaceId>,
    /// Where everything was drawn just before a release changed the model,
    /// for the relayout that follows to glide from.
    flip: Option<Snapshot>,
    /// The preview being dragged and where it would land, as last laid out.
    making_room: Option<(usize, usize)>,

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
            add: None,
            sizes: Vec::new(),
            canvas: Canvas::whole(Rectangle::default()),
            fingerprint: Fingerprint::default(),
            windows_hover: Lifts::new(profile),
            workspaces_hover: Lifts::new(profile),
            profile,
            window_glides: Glides::new(profile),
            preview_glides: Glides::new(profile),
            flip: None,
            making_room: None,
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

    pub fn bar(&self) -> &[Slot] {
        &self.bar
    }

    /// The '+' tile's box in the bar, before the bar's climb.
    pub fn add_tile(&self) -> Option<Rectangle<f64, Logical>> {
        self.add.map(|slot| slot.thumb)
    }

    fn view(&self) -> BarView<'_> {
        BarView {
            slots: &self.bar,
            add: self.add,
        }
    }

    /// Workspace `index`'s preview where it is drawn this frame: lifted under
    /// the pointer if it is being dragged, stepped aside if one is, and gliding
    /// if a relayout just moved it.
    pub fn preview_slot(&self, index: usize) -> Option<Slot> {
        let slot = *self.bar.get(index)?;
        if let Some(reorder) = self.input.reordering()
            && reorder.workspace == index
        {
            let thumb = reorder.rect();
            return Some(Slot {
                label: Rectangle::new(
                    (thumb.loc.x, thumb.loc.y + thumb.size.h).into(),
                    (thumb.size.w, slot.label.size.h).into(),
                ),
                thumb,
            });
        }
        let target = self.room_target(index).unwrap_or(slot);
        let glided = match self.workspace_id(index) {
            Some(id) => self.preview_glides.rect(id, target.thumb),
            None => target.thumb,
        };
        let shift = glided.loc - target.thumb.loc;
        Some(Slot {
            thumb: glided,
            label: Rectangle::new(target.label.loc + shift, target.label.size),
        })
    }

    /// Where preview `index` stands while another is dragged: the slot it
    /// steps into to make room for it.
    fn room_target(&self, index: usize) -> Option<Slot> {
        let (from, to) = self.making_room?;
        let step = match (from < to, from > to) {
            (true, _) if index > from && index <= to => index - 1,
            (_, true) if index >= to && index < from => index + 1,
            _ => return None,
        };
        self.bar.get(step).copied()
    }

    /// Which preview is lifted out of the bar, so the renderer can draw it
    /// above the rest.
    pub fn lifted_preview(&self) -> Option<usize> {
        self.input.reordering().map(|reorder| reorder.workspace)
    }

    /// How visible preview `index`'s '×' is: shown on the hovered preview,
    /// when there is more than one and none is being dragged.
    pub fn close_visibility(&self, index: usize) -> f64 {
        if self.bar.len() < 2 || self.input.reordering().is_some() {
            return 0.0;
        }
        self.workspaces_hover.at(index)
    }

    /// How far the pointer has lifted the '+' tile.
    pub fn add_lift(&self) -> f64 {
        self.workspaces_hover.at(self.bar.len())
    }

    fn workspace_id(&self, index: usize) -> Option<WorkspaceId> {
        self.pages.get(index).and_then(|page| page.workspace)
    }

    /// Where window `slot` of workspace `workspace` is drawn in the grid this
    /// frame: its cell, unless it is still gliding there.
    pub fn grid_rect(&self, workspace: usize, slot: usize) -> Option<Rectangle<f64, Logical>> {
        let page = self.pages.get(workspace)?;
        let cell = *page.grid.get(slot)?;
        Some(match page.windows.get(slot) {
            Some(id) => self.window_glides.rect(*id, cell),
            None => cell,
        })
    }

    /// Sends a window let go over nothing back to its cell, at the speed it
    /// was let go with.
    pub fn return_window(
        &mut self,
        index: usize,
        from: Rectangle<f64, Logical>,
        velocity: (f64, f64),
    ) {
        self.flip = None;
        let Some(page) = self.pages.get(self.active) else {
            return;
        };
        let (Some(id), Some(cell)) = (page.windows.get(index), page.grid.get(index)) else {
            return;
        };
        self.window_glides.launch(*id, from, *cell, velocity);
    }

    /// Where every window and preview is drawn right now — a carried window
    /// where the hand has it, not the cell it left.
    fn snapshot(&self) -> Snapshot {
        let carried = self.carrying();
        let windows = self
            .pages
            .iter()
            .enumerate()
            .flat_map(|(workspace, page)| {
                page.windows
                    .iter()
                    .enumerate()
                    .filter_map(move |(slot, id)| {
                        let rect = match carried {
                            Some((window, rect)) if workspace == self.active && window == slot => {
                                rect
                            }
                            _ => self.grid_rect(workspace, slot)?,
                        };
                        Some((*id, rect))
                    })
            })
            .collect();
        let previews = (0..self.bar.len())
            .filter_map(|index| Some((self.workspace_id(index)?, self.preview_slot(index)?.thumb)))
            .collect();
        Snapshot { windows, previews }
    }

    /// Glides everything that moved from where `before` had it to where the
    /// layout puts it now.
    fn glide_from(&mut self, before: Snapshot) {
        for (id, from) in before.windows {
            if let Some(to) = self.pages.iter().find_map(|page| {
                page.windows
                    .iter()
                    .position(|it| *it == id)
                    .map(|slot| page.grid[slot])
            }) {
                self.window_glides.launch(id, from, to, (0.0, 0.0));
            }
        }
        for (id, from) in before.previews {
            if let Some(to) = self
                .pages
                .iter()
                .position(|page| page.workspace == Some(id))
                .and_then(|index| self.bar.get(index))
            {
                self.preview_glides.launch(id, from, to.thumb, (0.0, 0.0));
            }
        }
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

        let slot = match self.input.hovered() {
            Some(Target::Workspace(workspace)) => self.bar.get(workspace).copied(),
            Some(Target::NewWorkspace) => self.add,
            _ => None,
        };
        let Some(slot) = slot else {
            return Some((carry.window, riding));
        };
        let Some(desktop) = self
            .pages
            .get(self.active)
            .and_then(|page| page.desktop.get(carry.window))
        else {
            return Some((carry.window, riding));
        };

        let landing = scene::inside(*desktop, self.canvas.usable, self.thumb(slot));
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

    /// The scale `window` of workspace `workspace` settles at while the
    /// overview is open: its grid cell on the active workspace, its
    /// workspace's preview in the bar otherwise.
    pub fn thumbnail_scale(&self, workspace: usize, window: WindowId) -> Option<f64> {
        let page = self.pages.get(workspace)?;
        if workspace != self.active {
            let preview = self.bar.get(workspace)?;
            return Some(preview.thumb.size.w / f64::from(self.canvas.usable.size.w.max(1)));
        }
        let slot = page.windows.iter().position(|id| *id == window)?;
        let cell = page.grid.get(slot)?;
        let desktop = page.desktop.get(slot)?;
        Some(cell.size.w / f64::from(desktop.size.w.max(1)))
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
        // Only an overview already on screen glides: one opening flies its
        // windows in from the desktop instead.
        let before = match self.overview.is_open() {
            true => Some(self.flip.take().unwrap_or_else(|| self.snapshot())),
            false => None,
        };
        self.canvas = Canvas::new(monitor.geometry(), monitor.usable());
        self.active = monitor.active_index();

        self.pages
            .resize_with(monitor.workspaces().len(), Page::default);

        for (workspace, page) in monitor.workspaces().iter().zip(&mut self.pages) {
            page.workspace = Some(workspace.id());
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

        // One slot more than there are workspaces: the '+' tile, laid out with
        // the previews so the bar stays centred with it in.
        scene::bar(
            self.canvas,
            monitor.workspaces().len() + 1,
            &self.metrics,
            &mut self.bar,
        );
        self.add = self.bar.pop();
        self.chrome.ensure(self.bar.len() + 1);
        self.windows_hover.resize(self.grid().len());
        self.workspaces_hover.resize(self.bar.len() + 1);
        self.making_room = None;

        if let Some(before) = before {
            self.glide_from(before);
        }
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
            || !self.window_glides.at_rest()
            || !self.preview_glides.at_rest()
    }

    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.profile = profile;
        self.overview.set_profile(profile);
        self.input.set_profile(profile);
        self.windows_hover.set_profile(profile);
        self.workspaces_hover.set_profile(profile);
        self.window_glides.set_profile(profile);
        self.preview_glides.set_profile(profile);
    }

    pub fn step(&mut self, dt: f32) {
        self.overview.step(dt);
        self.windows_hover.step(dt);
        self.workspaces_hover.step(dt);
        self.input.step(dt);
        self.window_glides.step(dt);
        self.preview_glides.step(dt);

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
        self.window_glides.settle();
        self.preview_glides.settle();
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
        let changed = input.motion(at, self.grid(), self.view());
        self.input = input;

        if changed {
            self.aim_hover();
        }
        self.make_room();
        changed
    }

    /// Steps the previews aside for one being dragged along the bar, gliding
    /// each from wherever it is to the place it now stands in.
    fn make_room(&mut self) {
        let room = self
            .input
            .reordering()
            .map(|reorder| (reorder.workspace, reorder.slot));
        if room == self.making_room {
            return;
        }
        let before: Vec<_> = (0..self.bar.len())
            .filter_map(|index| Some((index, self.preview_slot(index)?.thumb)))
            .collect();
        self.making_room = room;
        for (index, from) in before {
            let Some(id) = self.workspace_id(index) else {
                continue;
            };
            let Some(to) = self
                .room_target(index)
                .or_else(|| self.bar.get(index).copied())
            else {
                continue;
            };
            self.preview_glides.launch(id, from, to.thumb, (0.0, 0.0));
        }
    }

    pub fn press(&mut self, at: Point<f64, Logical>) -> Option<Target> {
        self.flip = None;
        let mut input = self.input;
        let target = input.press(at, self.grid(), self.view());
        self.input = input;
        self.aim_hover();
        target
    }

    /// Releases the pointer. Remembers where everything was drawn, so the
    /// relayout a model change brings glides from there rather than jumping.
    pub fn release(&mut self, at: Point<f64, Logical>) -> Release {
        self.flip = Some(self.snapshot());
        let mut input = self.input;
        let release = input.release(at, self.view());
        self.input = input;
        self.making_room = None;
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
            Some(Target::Workspace(index) | Target::Close(index)) => Some(index),
            Some(Target::NewWorkspace) => Some(self.bar.len()),
            _ => None,
        });
    }
}

/// Where every window and preview was drawn at one moment, by identity.
#[derive(Debug, Default)]
struct Snapshot {
    windows: Vec<(WindowId, Rectangle<f64, Logical>)>,
    previews: Vec<(WorkspaceId, Rectangle<f64, Logical>)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laid_out(windows: usize, workspaces: usize) -> SpaceControl {
        let mut space = SpaceControl::new();
        space.canvas = Canvas::whole(Rectangle::new((0, 0).into(), (1920, 1080).into()));
        space.pages = (0..workspaces)
            .map(|_| Page {
                workspace: Some(WorkspaceId::next()),
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
    fn a_grid_window_reports_its_cell_against_its_desktop_size() {
        let space = laid_out(2, 2);
        let window = space.pages[0].windows[1];
        assert_eq!(space.thumbnail_scale(0, window), Some(0.25));
    }

    #[test]
    fn a_window_on_another_workspace_reports_its_preview_scale() {
        let space = laid_out(2, 2);
        let window = space.pages[1].windows[0];
        let preview = space.bar()[1].thumb.size.w / 1920.0;
        assert_eq!(space.thumbnail_scale(1, window), Some(preview));
        assert!(preview < 0.25);
    }

    #[test]
    fn an_unknown_window_has_no_thumbnail() {
        let space = laid_out(2, 2);
        assert_eq!(space.thumbnail_scale(0, WindowId::next()), None);
        assert_eq!(space.thumbnail_scale(5, WindowId::next()), None);
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
    fn a_window_let_go_over_nothing_glides_back_to_its_cell() {
        let mut space = laid_out(2, 2);
        space.overview.snap_to(true);
        space.press((50.0, 150.0).into());
        space.motion((900.0, 500.0).into());
        for _ in 0..3 {
            space.step(1.0 / 60.0);
        }
        let Release::Returned {
            window,
            from,
            velocity,
        } = space.release((900.0, 500.0).into())
        else {
            panic!("expected a return");
        };
        space.return_window(window, from, velocity);
        let cell = space.pages[0].grid[0];
        assert_ne!(
            space.grid_rect(0, 0),
            Some(cell),
            "it starts where it was let go"
        );

        for _ in 0..240 {
            space.step(1.0 / 60.0);
        }
        assert_eq!(space.grid_rect(0, 0), Some(cell));
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

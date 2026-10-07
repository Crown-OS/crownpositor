//! One output's overview: its state machine, and the layout it is showing.
//!
//! The `spacecontrol` crate does not know what a [`Tile`] is, so this is where
//! the shell model is turned into the flat snapshots it works in — and where
//! the indices it hands back become window ids again.
//!
//! [`Tile`]: crate::shell::tile::Tile

mod glide;
mod ids;
mod layout;

pub use ids::{OverviewIds, TileIds};

use smithay::utils::{Logical, Point, Rectangle, Size};

use spacecontrol::{
    animations::{glide::Glides, hover::Lifts, spring::SpringProfile},
    interaction::{BarView, Interaction, Release, Target},
    overview::Overview,
    scene::{self, Canvas, Metrics, Slot},
};

use crate::{
    shell::monitor::Monitor,
    utils::id::{WindowId, WorkspaceId},
};
use glide::Snapshot;
use layout::{Fingerprint, Page};

/// The overview on one output.
#[derive(Debug)]
pub struct SpaceControl {
    overview: Overview,
    input: Interaction,
    ids: OverviewIds,
    metrics: Metrics,

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
            ids: OverviewIds::default(),
            metrics: Metrics::default(),
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
        }
    }

    pub fn overview(&self) -> &Overview {
        &self.overview
    }

    pub fn ids(&self) -> &OverviewIds {
        &self.ids
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
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

    /// Which preview is lifted out of the bar, so the renderer can draw it
    /// above the rest.
    pub fn lifted_preview(&self) -> Option<usize> {
        self.input.reordering().map(|reorder| reorder.workspace)
    }

    /// How visible preview `index`'s '×' is: shown on the hovered preview,
    /// when there is more than one and nothing is being dragged — a window
    /// carried over a preview is being dropped there, not removing it.
    pub fn close_visibility(&self, index: usize) -> f64 {
        let dragging = self.input.reordering().is_some() || self.input.carrying().is_some();
        if self.bar.len() < 2 || dragging {
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
        space.ids.ensure(workspaces);
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
    fn a_window_carried_over_a_preview_does_not_offer_to_close_it() {
        let mut space = laid_out(1, 3);
        space.overview.snap_to(true);
        space.press((50.0, 150.0).into());
        let slot = space.bar()[1].thumb;
        space.motion(
            (
                slot.loc.x + slot.size.w / 2.0,
                slot.loc.y + slot.size.h / 2.0,
            )
                .into(),
        );
        space.settle();

        assert_eq!(space.workspace_lift(1), 1.0, "the drop target still lifts");
        assert_eq!(space.close_visibility(1), 0.0);
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
}

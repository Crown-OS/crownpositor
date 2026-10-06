//! What the pointer is doing to the overview.
//!
//! Hit-testing, the hover highlight, dragging a window from the grid onto a
//! workspace, dragging a workspace preview along the bar to reorder it, and
//! the bar's '+' tile and '×' buttons. The overview's contents are handed in on
//! every call rather than stored, because the compositor already owns them and
//! a second copy here is a second thing to keep in step.
//!
//! A press is not a drag until the pointer has actually travelled: picking a
//! window with a shaky hand must still count as a click, so the decision waits
//! for [`DRAG_THRESHOLD`]. Whatever is picked up then has weight: it chases the
//! hand on a spring rather than being nailed to it, and the speed it has when
//! let go is handed on to wherever it goes next.

use smithay::utils::{Logical, Point, Rectangle};

use crate::{
    animations::spring::{Spring, SpringProfile},
    scene::{self, Slot},
};

/// How far the pointer must travel with the button down before the press stops
/// being a click and becomes a drag, in logical pixels.
const DRAG_THRESHOLD: f64 = 8.0;

/// How much a window shrinks while it is being carried, so it reads as picked
/// up off the grid rather than still part of it.
const CARRY_SCALE: f64 = 0.85;

/// How much a workspace preview grows while it is dragged along the bar.
const REORDER_LIFT: f64 = 1.08;

/// How the carried window goes into the preview under it. Stiffer than
/// anything the user can pick — this is direct manipulation, and a window that
/// trails the pointer into a workspace reads as lag rather than as motion.
/// Critically damped, so it never overshoots the preview it is going into.
const SNAP: SpringProfile = SpringProfile {
    stiffness: 900.0,
    damping: 60.0,
};

/// How a picked-up thing chases the hand: a little under critical damping, so
/// it has weight — it trails a fast flick and settles with the faintest
/// overshoot — while staying close enough to read as held.
const FOLLOW: SpringProfile = SpringProfile {
    stiffness: 900.0,
    damping: 48.0,
};

/// How a window shrinks as it leaves the grid.
const PICKUP: SpringProfile = SpringProfile::SNAPPY;

/// Something in the overview the pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A window in the grid, by its index in the grid rectangles.
    Window(usize),
    /// A workspace preview in the bottom bar, by its index.
    Workspace(usize),
    /// The '+' tile after the last preview.
    NewWorkspace,
    /// The '×' on workspace preview `n`.
    Close(usize),
}

/// The bar as the pointer sees it.
#[derive(Debug, Clone, Copy)]
pub struct BarView<'a> {
    pub slots: &'a [Slot],
    /// The '+' tile, if the bar has one.
    pub add: Option<Slot>,
}

impl BarView<'_> {
    /// Only a bar with more than one workspace offers to remove one.
    fn closable(&self) -> bool {
        self.slots.len() > 1
    }

    /// What a carried window would be dropped onto at `at`.
    fn drop_target(&self, at: Point<f64, Logical>) -> Option<Target> {
        if let Some(index) = self.slots.iter().position(|slot| slot.hit().contains(at)) {
            return Some(Target::Workspace(index));
        }
        self.add
            .filter(|add| add.thumb.contains(at))
            .map(|_| Target::NewWorkspace)
    }
}

/// A point chasing the pointer on a spring.
#[derive(Debug, Clone, Copy)]
struct Follow {
    x: Spring,
    y: Spring,
}

impl Follow {
    fn new(at: Point<f64, Logical>) -> Self {
        Self {
            x: Spring::with_profile(at.x as f32, FOLLOW),
            y: Spring::with_profile(at.y as f32, FOLLOW),
        }
    }

    fn aim(&mut self, at: Point<f64, Logical>, animated: bool) {
        self.x.set_target(at.x as f32);
        self.y.set_target(at.y as f32);
        if !animated {
            self.settle();
        }
    }

    fn at(&self) -> Point<f64, Logical> {
        Point::from((f64::from(self.x.position), f64::from(self.y.position)))
    }

    fn velocity(&self) -> (f64, f64) {
        (f64::from(self.x.velocity), f64::from(self.y.velocity))
    }

    fn step(&mut self, dt: f32) {
        self.x.step(dt);
        self.y.step(dt);
    }

    fn at_rest(&self) -> bool {
        self.x.at_rest() && self.y.at_rest()
    }

    fn settle(&mut self) {
        self.x.snap_to_target();
        self.y.snap_to_target();
    }
}

/// Where in a rectangle a press landed, as a fraction of its size — so the
/// rectangle keeps the same point under the pointer however it is resized.
fn grab_point(rect: Rectangle<f64, Logical>, at: Point<f64, Logical>) -> (f64, f64) {
    (
        ((at.x - rect.loc.x) / rect.size.w.max(1.0)).clamp(0.0, 1.0),
        ((at.y - rect.loc.y) / rect.size.h.max(1.0)).clamp(0.0, 1.0),
    )
}

/// `size` placed so the fraction `grab` of it sits on `at`.
fn hung_from(
    at: Point<f64, Logical>,
    grab: (f64, f64),
    size: (f64, f64),
) -> Rectangle<f64, Logical> {
    Rectangle::new(
        (at.x - size.0 * grab.0, at.y - size.1 * grab.1).into(),
        size.into(),
    )
}

/// A window in flight between the grid and a workspace preview.
#[derive(Debug, Clone, Copy)]
pub struct Carry {
    /// Index into the grid rectangles.
    pub window: usize,
    grab: (f64, f64),
    /// The thumbnail's size in the grid, which the carried copy shrinks from.
    size: (f64, f64),
    at: Point<f64, Logical>,
    follow: Follow,
    scale: Spring,
}

impl Carry {
    fn pick(
        window: usize,
        rect: Rectangle<f64, Logical>,
        from: Point<f64, Logical>,
        animated: bool,
    ) -> Self {
        let mut scale = Spring::with_profile(1.0, PICKUP);
        scale.set_target(CARRY_SCALE as f32);
        if !animated {
            scale.snap_to_target();
        }
        Self {
            window,
            grab: grab_point(rect, from),
            size: (rect.size.w, rect.size.h),
            at: from,
            follow: Follow::new(from),
            scale,
        }
    }

    /// Where to draw the carried window this frame.
    pub fn rect(&self) -> Rectangle<f64, Logical> {
        let scale = f64::from(self.scale.position);
        hung_from(
            self.follow.at(),
            self.grab,
            (self.size.0 * scale, self.size.1 * scale),
        )
    }

    /// Where the pointer is, which the window is on its way to.
    pub fn at(&self) -> Point<f64, Logical> {
        self.at
    }

    /// How fast the window is moving, in logical pixels per second.
    pub fn velocity(&self) -> (f64, f64) {
        self.follow.velocity()
    }

    fn aim(&mut self, at: Point<f64, Logical>, animated: bool) {
        self.at = at;
        self.follow.aim(at, animated);
    }

    fn step(&mut self, dt: f32) {
        self.follow.step(dt);
        self.scale.step(dt);
    }

    fn at_rest(&self) -> bool {
        self.follow.at_rest() && self.scale.at_rest()
    }

    fn settle(&mut self) {
        self.follow.settle();
        self.scale.snap_to_target();
    }
}

/// A workspace preview lifted out of the bar and dragged along it.
#[derive(Debug, Clone, Copy)]
pub struct Reorder {
    /// The preview's index when it was picked up.
    pub workspace: usize,
    /// Where it would land if let go now.
    pub slot: usize,
    grab: (f64, f64),
    size: (f64, f64),
    follow: Follow,
}

impl Reorder {
    fn pick(workspace: usize, thumb: Rectangle<f64, Logical>, from: Point<f64, Logical>) -> Self {
        Self {
            workspace,
            slot: workspace,
            grab: grab_point(thumb, from),
            size: (thumb.size.w, thumb.size.h),
            follow: Follow::new(from),
        }
    }

    /// Where to draw the lifted preview this frame.
    pub fn rect(&self) -> Rectangle<f64, Logical> {
        hung_from(
            self.follow.at(),
            self.grab,
            (self.size.0 * REORDER_LIFT, self.size.1 * REORDER_LIFT),
        )
    }
}

/// What a release turned out to mean.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Release {
    /// The pointer never travelled: this was a click on something.
    Click(Target),
    /// A window was carried onto a workspace preview and dropped there.
    Dropped { window: usize, workspace: usize },
    /// A window was carried onto the '+' tile: it goes to a new workspace.
    DroppedOnNew { window: usize },
    /// A window was carried and let go over nothing. It belongs back in the
    /// grid, and flies there from `from` at the speed it was let go with.
    Returned {
        window: usize,
        from: Rectangle<f64, Logical>,
        velocity: (f64, f64),
    },
    /// A workspace preview was dragged from one place in the bar to another,
    /// and let go at `from`.
    Reordered {
        from: usize,
        to: usize,
        rect: Rectangle<f64, Logical>,
    },
    /// The release meant nothing.
    None,
}

/// A press that has not been released yet.
#[derive(Debug, Clone, Copy)]
struct Press {
    target: Target,
    from: Point<f64, Logical>,
    carry: Option<Carry>,
    reorder: Option<Reorder>,
}

#[derive(Debug, Clone, Copy)]
pub struct Interaction {
    hovered: Option<Target>,
    press: Option<Press>,
    /// How far a carried window has settled into the preview it is over: 0 is
    /// riding under the pointer, 1 is sitting in the workspace it would land
    /// in. A spring rather than a swap so the window is seen to go in.
    snap: Spring,
    /// Whether anything moves at all, or lands on the frame it happens.
    animated: bool,
}

impl Default for Interaction {
    fn default() -> Self {
        Self::new()
    }
}

impl Interaction {
    pub fn new() -> Self {
        Self {
            hovered: None,
            press: None,
            snap: Spring::with_profile(0.0, SNAP),
            animated: true,
        }
    }

    /// Only whether things move at all is taken from the setting. Their feel
    /// is not the user's to choose: these are the speeds that keep a picked-up
    /// thing under the hand that is carrying it.
    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.animated = profile.is_some();
        if !self.animated {
            self.settle();
        }
    }

    /// How far the carried window has gone into the preview under it.
    pub fn snap(&self) -> f64 {
        f64::from(self.snap.position.clamp(0.0, 1.0))
    }

    pub fn step(&mut self, dt: f32) {
        self.snap.step(dt);
        if let Some(press) = &mut self.press {
            if let Some(carry) = &mut press.carry {
                carry.step(dt);
            }
            if let Some(reorder) = &mut press.reorder {
                reorder.follow.step(dt);
            }
        }
    }

    pub fn at_rest(&self) -> bool {
        self.snap.at_rest()
            && self.press.is_none_or(|press| {
                press.carry.is_none_or(|carry| carry.at_rest())
                    && press.reorder.is_none_or(|reorder| reorder.follow.at_rest())
            })
    }

    pub fn settle(&mut self) {
        self.snap.snap_to_target();
        if let Some(press) = &mut self.press {
            if let Some(carry) = &mut press.carry {
                carry.settle();
            }
            if let Some(reorder) = &mut press.reorder {
                reorder.follow.settle();
            }
        }
    }

    /// Aims the snap at wherever the carried window currently is, and lands it
    /// when motion is switched off.
    fn aim_snap(&mut self, over_preview: bool) {
        self.snap.set_target(f32::from(u8::from(over_preview)));
        if !self.animated {
            self.snap.snap_to_target();
        }
    }

    /// What the pointer is over, for the hover highlight. A window being
    /// carried does not highlight itself — it is in the air, not under the
    /// pointer.
    pub fn hovered(&self) -> Option<Target> {
        self.hovered
    }

    /// The window currently being carried, if any.
    pub fn carrying(&self) -> Option<Carry> {
        self.press.and_then(|press| press.carry)
    }

    /// The workspace preview currently being dragged along the bar, if any.
    pub fn reordering(&self) -> Option<Reorder> {
        self.press.and_then(|press| press.reorder)
    }

    /// Forgets everything. Called when the overview closes, so a press that was
    /// never released cannot survive into the next time it opens.
    pub fn clear(&mut self) {
        self.hovered = None;
        self.press = None;
        self.snap.hold(0.0);
    }

    /// Whatever is under `at`, topmost first. The bar wins over the grid
    /// because it is drawn over it, and a preview's '×' over the preview.
    pub fn target_at(
        at: Point<f64, Logical>,
        grid: &[Rectangle<f64, Logical>],
        bar: BarView<'_>,
    ) -> Option<Target> {
        if let Some(index) = bar.slots.iter().position(|slot| slot.hit().contains(at)) {
            if bar.closable() && scene::close_button(bar.slots[index].thumb).contains(at) {
                return Some(Target::Close(index));
            }
            return Some(Target::Workspace(index));
        }
        if bar.add.is_some_and(|add| add.thumb.contains(at)) {
            return Some(Target::NewWorkspace);
        }
        // Last drawn is topmost, so the search runs backwards.
        grid.iter()
            .rposition(|rect| rect.contains(at))
            .map(Target::Window)
    }

    /// Moves the pointer. Returns whether anything that needs redrawing
    /// changed, so a pointer wandering across empty space costs no frames.
    pub fn motion(
        &mut self,
        at: Point<f64, Logical>,
        grid: &[Rectangle<f64, Logical>],
        bar: BarView<'_>,
    ) -> bool {
        let animated = self.animated;
        if let Some(press) = &mut self.press {
            let travelled = at - press.from;
            if press.carry.is_none()
                && press.reorder.is_none()
                && travelled.x.hypot(travelled.y) >= DRAG_THRESHOLD
            {
                match press.target {
                    Target::Window(window) => {
                        press.carry = grid
                            .get(window)
                            .map(|rect| Carry::pick(window, *rect, press.from, animated));
                    }
                    Target::Workspace(workspace) => {
                        press.reorder = bar
                            .slots
                            .get(workspace)
                            .map(|slot| Reorder::pick(workspace, slot.thumb, press.from));
                    }
                    Target::NewWorkspace | Target::Close(_) => {}
                }
            }

            if let Some(carry) = &mut press.carry {
                carry.aim(at, animated);
                // While carrying, only the bar can light up — it is the only
                // thing a window can be dropped on.
                let over = bar.drop_target(at);
                self.hovered = over;
                self.aim_snap(over.is_some());
                return true;
            }
            if let Some(reorder) = &mut press.reorder {
                reorder.follow.aim(at, animated);
                reorder.slot = scene::nearest_slot(bar.slots, at.x).unwrap_or(reorder.workspace);
                self.hovered = None;
                return true;
            }
        }

        let over = Self::target_at(at, grid, bar);
        let changed = over != self.hovered;
        self.hovered = over;
        changed
    }

    /// Presses the button. Returns what was pressed, which is not yet a click:
    /// the caller should wait for [`Interaction::release`] to act.
    pub fn press(
        &mut self,
        at: Point<f64, Logical>,
        grid: &[Rectangle<f64, Logical>],
        bar: BarView<'_>,
    ) -> Option<Target> {
        let target = Self::target_at(at, grid, bar);
        self.press = target.map(|target| Press {
            target,
            from: at,
            carry: None,
            reorder: None,
        });
        self.hovered = target;
        self.snap.hold(0.0);
        target
    }

    /// Releases the button and says what the whole gesture meant.
    pub fn release(&mut self, at: Point<f64, Logical>, bar: BarView<'_>) -> Release {
        let Some(press) = self.press.take() else {
            return Release::None;
        };
        self.snap.hold(0.0);

        if let Some(reorder) = press.reorder {
            return Release::Reordered {
                from: reorder.workspace,
                to: reorder.slot,
                rect: reorder.rect(),
            };
        }
        let Some(carry) = press.carry else {
            return Release::Click(press.target);
        };

        match bar.drop_target(at) {
            Some(Target::Workspace(workspace)) => Release::Dropped {
                window: carry.window,
                workspace,
            },
            Some(Target::NewWorkspace) => Release::DroppedOnNew {
                window: carry.window,
            },
            _ => Release::Returned {
                window: carry.window,
                from: carry.rect(),
                velocity: carry.velocity(),
            },
        }
    }

    /// Abandons a press without acting on it — the overview closing under the
    /// pointer, or the pointer leaving the output.
    pub fn cancel(&mut self) -> Option<usize> {
        let carried = self.press.take().and_then(|press| press.carry);
        self.hovered = None;
        self.snap.hold(0.0);
        carried.map(|carry| carry.window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Vec<Rectangle<f64, Logical>> {
        vec![
            Rectangle::new((100.0, 100.0).into(), (200.0, 150.0).into()),
            Rectangle::new((400.0, 100.0).into(), (200.0, 150.0).into()),
        ]
    }

    fn slot(index: i32) -> Slot {
        let x = 100.0 + f64::from(index) * 250.0;
        Slot {
            thumb: Rectangle::new((x, 900.0).into(), (200.0, 112.0).into()),
            label: Rectangle::new((x, 1012.0).into(), (200.0, 24.0).into()),
        }
    }

    fn slots() -> Vec<Slot> {
        (0..3).map(slot).collect()
    }

    fn bar(slots: &[Slot]) -> BarView<'_> {
        BarView {
            slots,
            add: Some(slot(3)),
        }
    }

    fn at(x: f64, y: f64) -> Point<f64, Logical> {
        Point::from((x, y))
    }

    fn settle(input: &mut Interaction) {
        for _ in 0..240 {
            input.step(1.0 / 60.0);
        }
    }

    #[test]
    fn a_carried_window_is_in_the_preview_while_the_hand_is_still_over_it() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(150.0, 950.0), &grid(), bar(&slots));
        assert_eq!(input.snap(), 0.0, "it starts under the pointer");

        // A sixth of a second is the whole budget. Any slower and the window
        // is still on its way in when the hand has already let go.
        for _ in 0..10 {
            input.step(1.0 / 60.0);
        }
        assert!(input.snap() > 0.9, "the drop dawdled: {}", input.snap());
    }

    #[test]
    fn a_carried_window_comes_back_out_when_the_pointer_leaves() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(150.0, 950.0), &grid(), bar(&slots));
        for _ in 0..30 {
            input.step(1.0 / 60.0);
        }

        input.motion(at(700.0, 500.0), &grid(), bar(&slots));
        for _ in 0..30 {
            input.step(1.0 / 60.0);
        }
        assert!(input.snap() < 0.05, "{}", input.snap());
    }

    #[test]
    fn empty_space_is_over_nothing() {
        let slots = slots();
        assert_eq!(
            Interaction::target_at(at(50.0, 50.0), &grid(), bar(&slots)),
            None
        );
    }

    #[test]
    fn a_window_is_found_under_the_pointer() {
        let slots = slots();
        assert_eq!(
            Interaction::target_at(at(150.0, 150.0), &grid(), bar(&slots)),
            Some(Target::Window(0))
        );
        assert_eq!(
            Interaction::target_at(at(450.0, 150.0), &grid(), bar(&slots)),
            Some(Target::Window(1))
        );
    }

    #[test]
    fn a_workspace_preview_is_found_under_the_pointer() {
        let slots = slots();
        assert_eq!(
            Interaction::target_at(at(250.0, 990.0), &grid(), bar(&slots)),
            Some(Target::Workspace(0))
        );
        assert_eq!(
            Interaction::target_at(at(750.0, 990.0), &grid(), bar(&slots)),
            Some(Target::Workspace(2))
        );
    }

    #[test]
    fn a_label_is_as_good_a_target_as_its_preview() {
        let slots = slots();
        assert_eq!(
            Interaction::target_at(at(150.0, 1020.0), &grid(), bar(&slots)),
            Some(Target::Workspace(0))
        );
    }

    #[test]
    fn the_corner_of_a_preview_is_its_close_button() {
        let slots = slots();
        let button = scene::close_button(slots[1].thumb);
        let centre = at(
            button.loc.x + button.size.w / 2.0,
            button.loc.y + button.size.h / 2.0,
        );
        assert_eq!(
            Interaction::target_at(centre, &grid(), bar(&slots)),
            Some(Target::Close(1))
        );
    }

    #[test]
    fn the_last_workspace_cannot_be_closed() {
        let one = [slot(0)];
        let button = scene::close_button(one[0].thumb);
        assert_eq!(
            Interaction::target_at(button.loc + Point::from((1.0, 1.0)), &grid(), bar(&one)),
            Some(Target::Workspace(0))
        );
    }

    #[test]
    fn the_plus_tile_is_a_target() {
        let slots = slots();
        assert_eq!(
            Interaction::target_at(at(900.0, 950.0), &grid(), bar(&slots)),
            Some(Target::NewWorkspace)
        );
    }

    #[test]
    fn the_bar_wins_over_a_window_beneath_it() {
        let slots = slots();
        let overlapping = vec![Rectangle::new((100.0, 900.0).into(), (200.0, 112.0).into())];
        assert_eq!(
            Interaction::target_at(at(250.0, 990.0), &overlapping, bar(&slots)),
            Some(Target::Workspace(0))
        );
    }

    #[test]
    fn the_topmost_window_wins_where_two_overlap() {
        let slots = slots();
        let stacked = vec![
            Rectangle::new((100.0, 100.0).into(), (200.0, 150.0).into()),
            Rectangle::new((150.0, 120.0).into(), (200.0, 150.0).into()),
        ];
        assert_eq!(
            Interaction::target_at(at(200.0, 150.0), &stacked, bar(&slots)),
            Some(Target::Window(1))
        );
    }

    #[test]
    fn hovering_reports_only_real_changes() {
        let slots = slots();
        let mut input = Interaction::new();
        assert!(input.motion(at(150.0, 150.0), &grid(), bar(&slots)));
        assert_eq!(input.hovered(), Some(Target::Window(0)));

        assert!(
            !input.motion(at(160.0, 160.0), &grid(), bar(&slots)),
            "still the same window"
        );
        assert!(input.motion(at(450.0, 150.0), &grid(), bar(&slots)));
        assert_eq!(input.hovered(), Some(Target::Window(1)));

        assert!(input.motion(at(50.0, 50.0), &grid(), bar(&slots)));
        assert_eq!(input.hovered(), None);
    }

    #[test]
    fn a_press_and_release_in_place_is_a_click() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        assert_eq!(
            input.release(at(150.0, 150.0), bar(&slots)),
            Release::Click(Target::Window(0))
        );
    }

    #[test]
    fn a_shaky_hand_still_clicks() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(153.0, 152.0), &grid(), bar(&slots));

        assert!(input.carrying().is_none(), "three pixels is not a drag");
        assert_eq!(
            input.release(at(153.0, 152.0), bar(&slots)),
            Release::Click(Target::Window(0))
        );
    }

    #[test]
    fn clicking_a_workspace_preview_is_a_click() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(250.0, 990.0), &grid(), bar(&slots));
        assert_eq!(
            input.release(at(250.0, 990.0), bar(&slots)),
            Release::Click(Target::Workspace(0))
        );
    }

    #[test]
    fn pressing_empty_space_means_nothing() {
        let slots = slots();
        let mut input = Interaction::new();
        assert_eq!(input.press(at(50.0, 50.0), &grid(), bar(&slots)), None);
        assert_eq!(input.release(at(50.0, 50.0), bar(&slots)), Release::None);
    }

    #[test]
    fn travelling_far_enough_picks_the_window_up() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(300.0, 400.0), &grid(), bar(&slots));

        let carry = input.carrying().expect("the window should be in the air");
        assert_eq!(carry.window, 0);
        assert_eq!(carry.at(), at(300.0, 400.0));
    }

    #[test]
    fn a_carried_window_trails_the_hand_and_catches_up() {
        let slots = slots();
        let mut input = Interaction::new();
        // Pressed dead centre of window 0.
        input.press(at(200.0, 175.0), &grid(), bar(&slots));
        input.motion(at(700.0, 500.0), &grid(), bar(&slots));

        let centre = |rect: Rectangle<f64, Logical>| {
            (
                rect.loc.x + rect.size.w / 2.0,
                rect.loc.y + rect.size.h / 2.0,
            )
        };
        let lagging = centre(input.carrying().expect("carried").rect());
        assert!(lagging.0 < 700.0, "it has weight: {lagging:?}");

        settle(&mut input);
        let caught = centre(input.carrying().expect("carried").rect());
        assert!((caught.0 - 700.0).abs() < 0.01, "{caught:?}");
        assert!((caught.1 - 500.0).abs() < 0.01, "{caught:?}");
    }

    #[test]
    fn a_carried_window_shrinks_on_its_way_up() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(200.0, 175.0), &grid(), bar(&slots));
        input.motion(at(700.0, 500.0), &grid(), bar(&slots));
        assert_eq!(input.carrying().expect("carried").rect().size.w, 200.0);

        settle(&mut input);
        let rect = input.carrying().expect("carried").rect();
        assert!((rect.size.w - 200.0 * CARRY_SCALE).abs() < 0.01, "{rect:?}");
    }

    #[test]
    fn without_motion_a_carried_window_is_nailed_to_the_hand() {
        let slots = slots();
        let mut input = Interaction::new();
        input.set_profile(None);
        input.press(at(200.0, 175.0), &grid(), bar(&slots));
        input.motion(at(700.0, 500.0), &grid(), bar(&slots));

        let rect = input.carrying().expect("carried").rect();
        assert!(
            (rect.loc.x + rect.size.w / 2.0 - 700.0).abs() < 0.01,
            "{rect:?}"
        );
    }

    #[test]
    fn a_window_dropped_on_a_workspace_moves_there() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(650.0, 950.0), &grid(), bar(&slots));

        assert_eq!(
            input.release(at(650.0, 950.0), bar(&slots)),
            Release::Dropped {
                window: 0,
                workspace: 2
            }
        );
    }

    #[test]
    fn a_window_dropped_on_the_plus_tile_gets_a_new_workspace() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(900.0, 950.0), &grid(), bar(&slots));
        assert_eq!(input.hovered(), Some(Target::NewWorkspace));

        assert_eq!(
            input.release(at(900.0, 950.0), bar(&slots)),
            Release::DroppedOnNew { window: 0 }
        );
    }

    #[test]
    fn a_window_dropped_on_nothing_flies_back_at_the_speed_it_was_let_go() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(800.0, 500.0), &grid(), bar(&slots));
        input.step(1.0 / 60.0);

        match input.release(at(800.0, 500.0), bar(&slots)) {
            Release::Returned {
                window, velocity, ..
            } => {
                assert_eq!(window, 0);
                assert!(velocity.0 > 0.0, "it was moving right: {velocity:?}");
            }
            other => panic!("expected a return, got {other:?}"),
        }
    }

    #[test]
    fn only_the_bar_lights_up_while_carrying() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        // Dragged over the *other* window, which is not a drop target.
        input.motion(at(450.0, 150.0), &grid(), bar(&slots));
        assert_eq!(input.hovered(), None);

        input.motion(at(150.0, 950.0), &grid(), bar(&slots));
        assert_eq!(input.hovered(), Some(Target::Workspace(0)));
    }

    #[test]
    fn a_dragged_window_never_becomes_its_own_drop_target() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(160.0, 160.0), &grid(), bar(&slots));
        assert_eq!(input.hovered(), None, "the window is in the air");
    }

    #[test]
    fn dragging_a_preview_along_the_bar_reorders_it() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(250.0, 990.0), &grid(), bar(&slots));
        input.motion(at(760.0, 990.0), &grid(), bar(&slots));

        let reorder = input.reordering().expect("the preview is lifted");
        assert_eq!((reorder.workspace, reorder.slot), (0, 2));
        assert!(input.carrying().is_none());
        match input.release(at(760.0, 990.0), bar(&slots)) {
            Release::Reordered { from, to, .. } => assert_eq!((from, to), (0, 2)),
            other => panic!("expected a reorder, got {other:?}"),
        }
    }

    #[test]
    fn a_preview_dragged_back_to_its_place_lands_where_it_was() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(500.0, 990.0), &grid(), bar(&slots));
        input.motion(at(530.0, 990.0), &grid(), bar(&slots));
        assert_eq!(input.reordering().expect("lifted").slot, 1);
    }

    #[test]
    fn a_release_with_no_press_means_nothing() {
        let slots = slots();
        let mut input = Interaction::new();
        assert_eq!(input.release(at(150.0, 150.0), bar(&slots)), Release::None);
    }

    #[test]
    fn cancelling_hands_back_whatever_was_in_the_air() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(800.0, 500.0), &grid(), bar(&slots));

        assert_eq!(input.cancel(), Some(0));
        assert!(input.carrying().is_none());
        assert_eq!(input.hovered(), None);
        assert_eq!(input.release(at(800.0, 500.0), bar(&slots)), Release::None);
    }

    #[test]
    fn cancelling_a_plain_press_carries_nothing() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        assert_eq!(input.cancel(), None);
    }

    #[test]
    fn clearing_forgets_an_unreleased_press() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), bar(&slots));
        input.motion(at(800.0, 500.0), &grid(), bar(&slots));
        input.clear();

        assert!(input.carrying().is_none());
        assert_eq!(input.hovered(), None);
        assert_eq!(input.release(at(800.0, 500.0), bar(&slots)), Release::None);
    }

    #[test]
    fn a_window_that_vanished_mid_press_is_not_picked_up() {
        let slots = slots();
        let mut input = Interaction::new();
        input.press(at(450.0, 150.0), &grid(), bar(&slots));
        // The window closed; the grid is now shorter than its index.
        input.motion(at(800.0, 500.0), &grid()[..1], bar(&slots));

        assert!(input.carrying().is_none());
    }
}

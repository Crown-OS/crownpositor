//! What the pointer is doing to the overview.
//!
//! Hit-testing, the hover highlight, and dragging a window from the grid onto
//! another workspace. The overview's contents are handed in on every call
//! rather than stored, because the compositor already owns them and a second
//! copy here is a second thing to keep in step.
//!
//! A press is not a drag until the pointer has actually travelled: picking a
//! window with a shaky hand must still count as a click, so the decision waits
//! for [`DRAG_THRESHOLD`].

use smithay::utils::{Logical, Point, Rectangle};

use crate::{
    animations::spring::{Spring, SpringProfile},
    scene::Slot,
};

/// How far the pointer must travel with the button down before the press stops
/// being a click and becomes a drag, in logical pixels.
const DRAG_THRESHOLD: f64 = 8.0;

/// How much a window shrinks while it is being carried, so it reads as picked
/// up off the grid rather than still part of it.
const CARRY_SCALE: f64 = 0.85;

/// How the carried window goes into the preview under it. Stiffer than
/// anything the user can pick — this is direct manipulation, and a window that
/// trails the pointer into a workspace reads as lag rather than as motion.
/// Critically damped, so it never overshoots the preview it is going into.
const SNAP: SpringProfile = SpringProfile {
    stiffness: 900.0,
    damping: 60.0,
};

/// Something in the overview the pointer can be over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A window in the grid, by its index in the grid rectangles.
    Window(usize),
    /// A workspace preview in the bottom bar, by its index.
    Workspace(usize),
}

/// A window in flight between the grid and a workspace preview.
#[derive(Debug, Clone, Copy)]
pub struct Carry {
    /// Index into the grid rectangles.
    pub window: usize,
    /// Where in the thumbnail it was picked up, as a fraction of its size — so
    /// the window keeps the same point under the pointer however much it
    /// shrinks on the way.
    grab: (f64, f64),
    /// The thumbnail's size in the grid, which the carried copy shrinks from.
    size: (f64, f64),
    at: Point<f64, Logical>,
}

impl Carry {
    /// Where to draw the carried window this frame.
    pub fn rect(&self) -> Rectangle<f64, Logical> {
        let (width, height) = (self.size.0 * CARRY_SCALE, self.size.1 * CARRY_SCALE);
        Rectangle::new(
            (
                self.at.x - width * self.grab.0,
                self.at.y - height * self.grab.1,
            )
                .into(),
            (width, height).into(),
        )
    }

    pub fn at(&self) -> Point<f64, Logical> {
        self.at
    }
}

/// What a release turned out to mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// The pointer never travelled: this was a click on something.
    Click(Target),
    /// A window was carried onto a workspace preview and dropped there.
    Dropped { window: usize, workspace: usize },
    /// A window was carried and let go over nothing; it belongs back in the
    /// grid it came from.
    Returned { window: usize },
    /// The release meant nothing.
    None,
}

/// A press that has not been released yet.
#[derive(Debug, Clone, Copy)]
struct Press {
    target: Target,
    from: Point<f64, Logical>,
    carry: Option<Carry>,
}

#[derive(Debug, Clone, Copy)]
pub struct Interaction {
    hovered: Option<Target>,
    press: Option<Press>,
    /// How far a carried window has settled into the preview it is over: 0 is
    /// riding under the pointer, 1 is sitting in the workspace it would land
    /// in. A spring rather than a swap so the window is seen to go in.
    snap: Spring,
    /// Whether the drop moves at all, or lands on the frame it happens.
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

    /// Only whether the drop moves at all is taken from the setting. Its feel
    /// is not the user's to choose: [`SNAP`] is the one speed that keeps the
    /// window under the hand that is carrying it.
    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.animated = profile.is_some();
        if !self.animated {
            self.snap.snap_to_target();
        }
    }

    /// How far the carried window has gone into the preview under it.
    pub fn snap(&self) -> f64 {
        f64::from(self.snap.position.clamp(0.0, 1.0))
    }

    pub fn step(&mut self, dt: f32) {
        self.snap.step(dt);
    }

    pub fn at_rest(&self) -> bool {
        self.snap.at_rest()
    }

    pub fn settle(&mut self) {
        self.snap.snap_to_target();
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

    /// Forgets everything. Called when the overview closes, so a press that was
    /// never released cannot survive into the next time it opens.
    pub fn clear(&mut self) {
        self.hovered = None;
        self.press = None;
        self.snap.hold(0.0);
    }

    /// Whatever is under `at`, topmost first. The bar wins over the grid
    /// because it is drawn over it.
    pub fn target_at(
        at: Point<f64, Logical>,
        grid: &[Rectangle<f64, Logical>],
        bar: &[Slot],
    ) -> Option<Target> {
        if let Some(index) = bar.iter().position(|slot| slot.hit().contains(at)) {
            return Some(Target::Workspace(index));
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
        bar: &[Slot],
    ) -> bool {
        if let Some(press) = &mut self.press {
            // Far enough to mean it: pick the window up.
            let travelled = at - press.from;
            if press.carry.is_none()
                && travelled.x.hypot(travelled.y) >= DRAG_THRESHOLD
                && let Target::Window(window) = press.target
                && let Some(rect) = grid.get(window)
            {
                press.carry = Some(Carry {
                    window,
                    grab: (
                        ((press.from.x - rect.loc.x) / rect.size.w.max(1.0)).clamp(0.0, 1.0),
                        ((press.from.y - rect.loc.y) / rect.size.h.max(1.0)).clamp(0.0, 1.0),
                    ),
                    size: (rect.size.w, rect.size.h),
                    at,
                });
            }

            if let Some(carry) = &mut press.carry {
                carry.at = at;
                // While carrying, only the bar can light up — it is the only
                // thing a window can be dropped on.
                let over = bar
                    .iter()
                    .position(|slot| slot.hit().contains(at))
                    .map(Target::Workspace);
                self.hovered = over;
                self.aim_snap(over.is_some());
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
        bar: &[Slot],
    ) -> Option<Target> {
        let target = Self::target_at(at, grid, bar);
        self.press = target.map(|target| Press {
            target,
            from: at,
            carry: None,
        });
        self.hovered = target;
        self.snap.hold(0.0);
        target
    }

    /// Releases the button and says what the whole gesture meant.
    pub fn release(&mut self, at: Point<f64, Logical>, bar: &[Slot]) -> Release {
        let Some(press) = self.press.take() else {
            return Release::None;
        };
        self.snap.hold(0.0);

        let Some(carry) = press.carry else {
            return Release::Click(press.target);
        };

        match bar.iter().position(|slot| slot.hit().contains(at)) {
            Some(workspace) => Release::Dropped {
                window: carry.window,
                workspace,
            },
            None => Release::Returned {
                window: carry.window,
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

    fn bar() -> Vec<Slot> {
        (0..3)
            .map(|index| {
                let x = 100.0 + f64::from(index) * 250.0;
                Slot {
                    thumb: Rectangle::new((x, 900.0).into(), (200.0, 112.0).into()),
                    label: Rectangle::new((x, 1012.0).into(), (200.0, 24.0).into()),
                }
            })
            .collect()
    }

    fn at(x: f64, y: f64) -> Point<f64, Logical> {
        Point::from((x, y))
    }

    #[test]
    fn a_carried_window_is_in_the_preview_while_the_hand_is_still_over_it() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(150.0, 950.0), &grid(), &bar());
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
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(150.0, 950.0), &grid(), &bar());
        for _ in 0..30 {
            input.step(1.0 / 60.0);
        }

        input.motion(at(700.0, 500.0), &grid(), &bar());
        for _ in 0..30 {
            input.step(1.0 / 60.0);
        }
        assert!(input.snap() < 0.05, "{}", input.snap());
    }

    #[test]
    fn empty_space_is_over_nothing() {
        assert_eq!(
            Interaction::target_at(at(50.0, 50.0), &grid(), &bar()),
            None
        );
    }

    #[test]
    fn a_window_is_found_under_the_pointer() {
        assert_eq!(
            Interaction::target_at(at(150.0, 150.0), &grid(), &bar()),
            Some(Target::Window(0))
        );
        assert_eq!(
            Interaction::target_at(at(450.0, 150.0), &grid(), &bar()),
            Some(Target::Window(1))
        );
    }

    #[test]
    fn a_workspace_preview_is_found_under_the_pointer() {
        assert_eq!(
            Interaction::target_at(at(150.0, 950.0), &grid(), &bar()),
            Some(Target::Workspace(0))
        );
        assert_eq!(
            Interaction::target_at(at(650.0, 950.0), &grid(), &bar()),
            Some(Target::Workspace(2))
        );
    }

    #[test]
    fn a_label_is_as_good_a_target_as_its_preview() {
        assert_eq!(
            Interaction::target_at(at(150.0, 1020.0), &grid(), &bar()),
            Some(Target::Workspace(0))
        );
    }

    #[test]
    fn the_bar_wins_over_a_window_beneath_it() {
        let overlapping = vec![Rectangle::new((100.0, 900.0).into(), (200.0, 112.0).into())];
        assert_eq!(
            Interaction::target_at(at(150.0, 950.0), &overlapping, &bar()),
            Some(Target::Workspace(0))
        );
    }

    #[test]
    fn the_topmost_window_wins_where_two_overlap() {
        let stacked = vec![
            Rectangle::new((100.0, 100.0).into(), (200.0, 150.0).into()),
            Rectangle::new((150.0, 120.0).into(), (200.0, 150.0).into()),
        ];
        assert_eq!(
            Interaction::target_at(at(200.0, 150.0), &stacked, &bar()),
            Some(Target::Window(1))
        );
    }

    #[test]
    fn hovering_reports_only_real_changes() {
        let mut input = Interaction::new();
        assert!(input.motion(at(150.0, 150.0), &grid(), &bar()));
        assert_eq!(input.hovered(), Some(Target::Window(0)));

        assert!(
            !input.motion(at(160.0, 160.0), &grid(), &bar()),
            "still the same window"
        );
        assert!(input.motion(at(450.0, 150.0), &grid(), &bar()));
        assert_eq!(input.hovered(), Some(Target::Window(1)));

        assert!(input.motion(at(50.0, 50.0), &grid(), &bar()));
        assert_eq!(input.hovered(), None);
    }

    #[test]
    fn a_press_and_release_in_place_is_a_click() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        assert_eq!(
            input.release(at(150.0, 150.0), &bar()),
            Release::Click(Target::Window(0))
        );
    }

    #[test]
    fn a_shaky_hand_still_clicks() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(153.0, 152.0), &grid(), &bar());

        assert!(input.carrying().is_none(), "three pixels is not a drag");
        assert_eq!(
            input.release(at(153.0, 152.0), &bar()),
            Release::Click(Target::Window(0))
        );
    }

    #[test]
    fn clicking_a_workspace_preview_is_a_click() {
        let mut input = Interaction::new();
        input.press(at(150.0, 950.0), &grid(), &bar());
        assert_eq!(
            input.release(at(150.0, 950.0), &bar()),
            Release::Click(Target::Workspace(0))
        );
    }

    #[test]
    fn pressing_empty_space_means_nothing() {
        let mut input = Interaction::new();
        assert_eq!(input.press(at(50.0, 50.0), &grid(), &bar()), None);
        assert_eq!(input.release(at(50.0, 50.0), &bar()), Release::None);
    }

    #[test]
    fn travelling_far_enough_picks_the_window_up() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(300.0, 400.0), &grid(), &bar());

        let carry = input.carrying().expect("the window should be in the air");
        assert_eq!(carry.window, 0);
        assert_eq!(carry.at(), at(300.0, 400.0));
    }

    #[test]
    fn a_carried_window_keeps_its_grab_point_under_the_pointer() {
        let mut input = Interaction::new();
        // Pressed dead centre of window 0.
        input.press(at(200.0, 175.0), &grid(), &bar());
        input.motion(at(700.0, 500.0), &grid(), &bar());

        let rect = input.carrying().expect("carried").rect();
        let centre = (
            rect.loc.x + rect.size.w / 2.0,
            rect.loc.y + rect.size.h / 2.0,
        );
        assert!((centre.0 - 700.0).abs() < 1e-6, "{centre:?}");
        assert!((centre.1 - 500.0).abs() < 1e-6, "{centre:?}");
    }

    #[test]
    fn a_carried_window_shrinks() {
        let mut input = Interaction::new();
        input.press(at(200.0, 175.0), &grid(), &bar());
        input.motion(at(700.0, 500.0), &grid(), &bar());

        let rect = input.carrying().expect("carried").rect();
        assert!((rect.size.w - 200.0 * CARRY_SCALE).abs() < 1e-6, "{rect:?}");
        assert!(rect.size.w < 200.0);
    }

    #[test]
    fn a_window_dropped_on_a_workspace_moves_there() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(650.0, 950.0), &grid(), &bar());

        assert_eq!(
            input.release(at(650.0, 950.0), &bar()),
            Release::Dropped {
                window: 0,
                workspace: 2
            }
        );
    }

    #[test]
    fn a_window_dropped_on_nothing_goes_back() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(800.0, 500.0), &grid(), &bar());

        assert_eq!(
            input.release(at(800.0, 500.0), &bar()),
            Release::Returned { window: 0 }
        );
    }

    #[test]
    fn only_the_bar_lights_up_while_carrying() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        // Dragged over the *other* window, which is not a drop target.
        input.motion(at(450.0, 150.0), &grid(), &bar());
        assert_eq!(input.hovered(), None);

        input.motion(at(150.0, 950.0), &grid(), &bar());
        assert_eq!(input.hovered(), Some(Target::Workspace(0)));
    }

    #[test]
    fn a_dragged_window_never_becomes_its_own_drop_target() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(160.0, 160.0), &grid(), &bar());
        assert_eq!(input.hovered(), None, "the window is in the air");
    }

    #[test]
    fn a_release_with_no_press_means_nothing() {
        let mut input = Interaction::new();
        assert_eq!(input.release(at(150.0, 150.0), &bar()), Release::None);
    }

    #[test]
    fn cancelling_hands_back_whatever_was_in_the_air() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(800.0, 500.0), &grid(), &bar());

        assert_eq!(input.cancel(), Some(0));
        assert!(input.carrying().is_none());
        assert_eq!(input.hovered(), None);
        assert_eq!(input.release(at(800.0, 500.0), &bar()), Release::None);
    }

    #[test]
    fn cancelling_a_plain_press_carries_nothing() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        assert_eq!(input.cancel(), None);
    }

    #[test]
    fn clearing_forgets_an_unreleased_press() {
        let mut input = Interaction::new();
        input.press(at(150.0, 150.0), &grid(), &bar());
        input.motion(at(800.0, 500.0), &grid(), &bar());
        input.clear();

        assert!(input.carrying().is_none());
        assert_eq!(input.hovered(), None);
        assert_eq!(input.release(at(800.0, 500.0), &bar()), Release::None);
    }

    #[test]
    fn a_window_that_vanished_mid_press_is_not_picked_up() {
        let mut input = Interaction::new();
        input.press(at(450.0, 150.0), &grid(), &bar());
        // The window closed; the grid is now shorter than its index.
        input.motion(at(800.0, 500.0), &grid()[..1], &bar());

        assert!(input.carrying().is_none());
    }
}

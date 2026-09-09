//! Where a window's frame sits relative to the window itself.
//!
//! The frame is *outside* the client area, the way macOS and Windows draw one:
//! a decorated window's rect is the titlebar plus the surface, and the client is
//! configured with what is left. Two rects therefore exist for every tile, and
//! confusing them is the whole bug surface of server-side decoration — so the
//! conversion lives here, in one place, with no renderer and no `WlSurface`
//! anywhere near it.

pub mod menu_layout;

use std::cell::Cell;

use smithay::{
    backend::renderer::{element::Id, utils::CommitCounter},
    utils::{Logical, Point, Rectangle, Size},
};

/// The render elements one window's frame is built from.
///
/// Their ids are generated once per window and kept: regenerating them per
/// frame would have the damage tracker treat every frame's decoration as a
/// brand new element and repaint the whole frame continuously.
#[derive(Debug, Clone)]
pub struct DecorationIds {
    /// The blurred glass behind the titlebar.
    pub glass: Id,
    /// The tinted panel and its controls.
    pub panel: Id,
    /// The hairline around the whole window.
    pub border: Id,
}

impl Default for DecorationIds {
    fn default() -> Self {
        Self {
            glass: Id::new(),
            panel: Id::new(),
            border: Id::new(),
        }
    }
}

/// Tracks whether a frame's *pixels* changed, as opposed to its geometry.
///
/// Moving and resizing are the damage tracker's own job. What it cannot see is
/// a focus change or the pointer crossing a control, because those change only
/// the uniforms — so the frame reports a new commit exactly when the appearance
/// it was last drawn with no longer matches.
///
/// A `Cell` because the render pass holds a `&Tile`: the counter is per-window
/// bookkeeping for the renderer, not shell state anything else can observe.
#[derive(Debug, Default)]
pub struct DecorationCommit {
    appearance: Cell<Option<u64>>,
    commit: Cell<CommitCounter>,
}

impl DecorationCommit {
    /// The counter to report for a frame drawn with this appearance.
    pub fn get(&self, appearance: u64) -> CommitCounter {
        if self.appearance.get() != Some(appearance) {
            self.appearance.set(Some(appearance));
            let mut next = self.commit.get();
            next.increment();
            self.commit.set(next);
        }
        self.commit.get()
    }
}

/// Diameter of a window control, in logical pixels.
const CONTROL_DIAMETER: i32 = 16;
/// Centre-to-centre distance between two controls.
const CONTROL_PITCH: i32 = 26;
/// From the frame's right edge to the centre of the rightmost control.
const CONTROL_MARGIN: i32 = 15;
/// From the frame's left edge to the start of the title.
const LABEL_MARGIN: i32 = 16;
/// Clear space between the title and the first control.
const LABEL_GUTTER: i32 = 12;
/// Space between the title and the menu row, and between menu items.
const MENU_GUTTER: i32 = 14;

/// A window control, in the order the titlebar draws them: left to right.
///
/// The discriminants are the indices the titlebar shader draws its glyphs by,
/// so the picture and the hit test cannot disagree about which is which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Control {
    Maximize,
    Minimize,
    Close,
}

impl Control {
    pub const ALL: [Self; 3] = [Self::Maximize, Self::Minimize, Self::Close];

    pub fn index(self) -> usize {
        match self {
            Self::Maximize => 0,
            Self::Minimize => 1,
            Self::Close => 2,
        }
    }
}

/// Where the pieces of a titlebar sit inside a window's frame.
///
/// Derived from the frame and the insets alone, so the renderer and the hit
/// test are looking at the same rectangles by construction rather than by two
/// people remembering the same constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleBarLayout {
    bar: Rectangle<i32, Logical>,
    /// Centre of the leftmost control.
    first_control: Point<i32, Logical>,
}

impl TitleBarLayout {
    pub fn new(frame: Rectangle<i32, Logical>, insets: Insets) -> Self {
        let bar = Rectangle::new(frame.loc, Size::from((frame.size.w, insets.top)));
        let rightmost = frame.loc.x + frame.size.w - CONTROL_MARGIN;

        Self {
            bar,
            first_control: Point::from((
                rightmost - CONTROL_PITCH * (Control::ALL.len() as i32 - 1),
                frame.loc.y + insets.top / 2,
            )),
        }
    }

    pub fn bar(&self) -> Rectangle<i32, Logical> {
        self.bar
    }

    pub fn control_radius(&self) -> i32 {
        CONTROL_DIAMETER / 2
    }

    pub fn control_pitch(&self) -> i32 {
        CONTROL_PITCH
    }

    pub fn first_control_centre(&self) -> Point<i32, Logical> {
        self.first_control
    }

    pub fn control_centre(&self, control: Control) -> Point<i32, Logical> {
        self.first_control + Point::from((CONTROL_PITCH * control.index() as i32, 0))
    }

    /// The clickable square around a control.
    ///
    /// Deliberately the full pitch wide rather than the disc's own diameter:
    /// the discs are small, and a hit target you have to aim at is a bad one.
    pub fn control_rect(&self, control: Control) -> Rectangle<i32, Logical> {
        let centre = self.control_centre(control);
        let half = CONTROL_PITCH / 2;
        Rectangle::new(
            Point::from((centre.x - half, self.bar.loc.y)),
            Size::from((CONTROL_PITCH, self.bar.size.h)),
        )
    }

    pub fn control_at(&self, point: Point<f64, Logical>) -> Option<Control> {
        Control::ALL
            .into_iter()
            .find(|control| self.control_rect(*control).to_f64().contains(point))
    }

    /// Where the title starts, and how much room it has before the controls.
    pub fn label(&self) -> Rectangle<i32, Logical> {
        let start = self.bar.loc.x + LABEL_MARGIN;
        let end = self.control_centre(Control::Maximize).x - CONTROL_PITCH / 2 - LABEL_GUTTER;
        Rectangle::new(
            Point::from((start, self.bar.loc.y)),
            Size::from(((end - start).max(0), self.bar.size.h)),
        )
    }

    /// Where the menu row starts, given how wide the title turned out.
    ///
    /// The title's width is measured, not assumed, so this takes it rather
    /// than guessing — and the row is whatever is left before the controls.
    pub fn menu(&self, label_width: i32) -> Rectangle<i32, Logical> {
        let label = self.label();
        let start = (label.loc.x + label_width + MENU_GUTTER).min(label.loc.x + label.size.w);
        Rectangle::new(
            Point::from((start, self.bar.loc.y)),
            Size::from(((label.loc.x + label.size.w - start).max(0), self.bar.size.h)),
        )
    }

    pub fn menu_gutter(&self) -> i32 {
        MENU_GUTTER
    }
}

/// The band the decoration occupies along each edge of a window.
///
/// Only the top is non-zero today. It is still a struct rather than a bare
/// `i32` because the resize band and a future bottom bar both belong here, and
/// every call site already asks the same two questions of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Insets {
    pub top: i32,
}

impl Insets {
    pub const NONE: Self = Self { top: 0 };

    pub fn top(height: i32) -> Self {
        Self { top: height.max(0) }
    }

    pub fn is_empty(self) -> bool {
        self.top == 0
    }

    /// The client area inside a frame.
    ///
    /// Never smaller than a pixel: a window dragged down to nothing would
    /// otherwise be handed a zero or negative size, which no client can honour
    /// and the renderer must never be given.
    pub fn content(self, frame: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        Rectangle::new(
            Point::from((frame.loc.x, frame.loc.y + self.top)),
            Size::from((frame.size.w.max(1), (frame.size.h - self.top).max(1))),
        )
    }

    /// The frame around a client area — the inverse of [`content`].
    ///
    /// This is the direction a client-driven size travels: the client says how
    /// big it wants to be, and the frame is that plus the titlebar.
    ///
    /// [`content`]: Self::content
    pub fn frame(self, content: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        Rectangle::new(
            Point::from((content.loc.x, content.loc.y - self.top)),
            Size::from((content.size.w, content.size.h + self.top)),
        )
    }

    /// The client size a frame of this size leaves room for.
    pub fn content_size(self, frame: Size<i32, Logical>) -> Size<i32, Logical> {
        Size::from((frame.w.max(1), (frame.h - self.top).max(1)))
    }

    /// The frame size a client of this size needs.
    pub fn frame_size(self, content: Size<i32, Logical>) -> Size<i32, Logical> {
        Size::from((content.w, content.h + self.top))
    }

    /// Whether an output-local point is in the decoration band of `frame`.
    pub fn contains(self, frame: Rectangle<i32, Logical>, point: Point<f64, Logical>) -> bool {
        !self.is_empty()
            && Rectangle::new(frame.loc, Size::from((frame.size.w, self.top)))
                .to_f64()
                .contains(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn the_content_sits_below_the_titlebar() {
        let insets = Insets::top(36);
        assert_eq!(
            insets.content(rect(10, 20, 800, 600)),
            rect(10, 56, 800, 564)
        );
    }

    /// The pair has to round-trip, or a window grows or shrinks by the
    /// titlebar's height every time it changes state.
    #[test]
    fn frame_and_content_are_inverses() {
        let insets = Insets::top(36);
        let frame = rect(10, 20, 800, 600);
        assert_eq!(insets.frame(insets.content(frame)), frame);
    }

    #[test]
    fn no_decoration_leaves_the_rect_alone() {
        let frame = rect(10, 20, 800, 600);
        assert_eq!(Insets::NONE.content(frame), frame);
        assert_eq!(Insets::NONE.frame(frame), frame);
    }

    /// A window mid-animation can be shorter than its own titlebar.
    #[test]
    fn the_content_never_collapses_to_nothing() {
        let content = Insets::top(36).content(rect(0, 0, 100, 10));
        assert!(content.size.w >= 1 && content.size.h >= 1);
    }

    #[test]
    fn the_size_pair_round_trips_too() {
        let insets = Insets::top(36);
        let content = Size::from((800, 564));
        assert_eq!(insets.content_size(insets.frame_size(content)), content);
    }

    #[test]
    fn a_frame_only_reports_a_new_commit_when_its_appearance_changes() {
        let tracker = DecorationCommit::default();

        let first = tracker.get(1);
        assert_eq!(tracker.get(1), first, "same appearance, same commit");

        let second = tracker.get(2);
        assert_ne!(second, first, "a focus or hover change has to repaint");
    }

    #[test]
    fn every_frame_element_gets_its_own_id() {
        let ids = DecorationIds::default();
        let all = [&ids.glass, &ids.panel, &ids.border];
        for (index, id) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert_ne!(id, other, "two elements sharing an id fight over damage");
            }
        }
    }

    #[test]
    fn the_controls_sit_at_the_right_hand_end_in_drawing_order() {
        let layout = TitleBarLayout::new(rect(0, 0, 800, 600), Insets::top(36));

        let centres = Control::ALL.map(|control| layout.control_centre(control).x);
        assert!(
            centres[0] < centres[1] && centres[1] < centres[2],
            "maximize, minimize, close run left to right"
        );
        assert_eq!(centres[2], 800 - CONTROL_MARGIN, "close is the rightmost");
        assert_eq!(layout.first_control_centre().x, centres[0]);
    }

    #[test]
    fn a_control_is_vertically_centred_in_the_bar() {
        let layout = TitleBarLayout::new(rect(0, 0, 800, 600), Insets::top(36));
        assert_eq!(layout.control_centre(Control::Close).y, 18);
    }

    #[test]
    fn clicking_a_control_finds_it_and_only_it() {
        let layout = TitleBarLayout::new(rect(0, 0, 800, 600), Insets::top(36));

        for control in Control::ALL {
            let centre = layout.control_centre(control).to_f64();
            assert_eq!(layout.control_at(centre), Some(control));
        }
        assert_eq!(
            layout.control_at((100.0, 18.0).into()),
            None,
            "in the title"
        );
        assert_eq!(
            layout.control_at((400.0, 90.0).into()),
            None,
            "below the bar"
        );
    }

    #[test]
    fn the_title_stops_short_of_the_controls() {
        let layout = TitleBarLayout::new(rect(0, 0, 800, 600), Insets::top(36));
        let label = layout.label();
        let first = layout.control_rect(Control::Maximize);

        assert_eq!(label.loc.x, LABEL_MARGIN);
        assert!(label.loc.x + label.size.w <= first.loc.x);
    }

    /// A window too narrow for both cannot be allowed to produce a negative
    /// width, which every rect consumer would then read as inverted.
    #[test]
    fn a_narrow_window_yields_an_empty_title_rather_than_a_negative_one() {
        let layout = TitleBarLayout::new(rect(0, 0, 60, 600), Insets::top(36));
        assert_eq!(layout.label().size.w, 0);
        assert_eq!(layout.menu(200).size.w, 0);
    }

    #[test]
    fn the_menu_row_follows_the_measured_title() {
        let layout = TitleBarLayout::new(rect(0, 0, 800, 600), Insets::top(36));
        let menu = layout.menu(64);

        assert_eq!(menu.loc.x, LABEL_MARGIN + 64 + MENU_GUTTER);
        assert!(menu.size.w > 0);
    }

    /// A title long enough to fill the bar leaves no menu row rather than one
    /// that overlaps the controls.
    #[test]
    fn an_overlong_title_squeezes_the_menu_out() {
        let layout = TitleBarLayout::new(rect(0, 0, 800, 600), Insets::top(36));
        assert_eq!(layout.menu(10_000).size.w, 0);
    }

    #[test]
    fn the_band_answers_for_the_titlebar_only() {
        let insets = Insets::top(36);
        let frame = rect(0, 0, 800, 600);
        assert!(insets.contains(frame, (400.0, 10.0).into()));
        assert!(
            !insets.contains(frame, (400.0, 40.0).into()),
            "in the client"
        );
        assert!(
            !insets.contains(frame, (900.0, 10.0).into()),
            "past the edge"
        );
        assert!(!Insets::NONE.contains(frame, (400.0, 0.0).into()));
    }
}

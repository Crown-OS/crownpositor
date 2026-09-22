//! Where everything sits on screen, for one output, at one moment.
//!
//! The overview is two regions: a grid of the active workspace's windows
//! filling most of the output, and a bar of workspace thumbnails along the
//! bottom. This module owns the arithmetic that divides the output between
//! them and places their contents; it decides no policy and holds no state.
//!
//! Every rectangle here is the *destination* — where a window has arrived once
//! the overview is fully open. [`between`] carries a window from where it
//! actually lives to that destination, which is what makes the entry animation
//! a property of the geometry rather than a special case in the renderer.

use smithay::utils::{Logical, Rectangle, Size};

use crate::layout;

/// The output the overview is drawn on.
///
/// Two rectangles rather than one: proportions are measured against the whole
/// output so the overview looks the same on every panel, but everything is
/// *placed* inside the part of it nothing has reserved — a top bar's strip is
/// not the overview's to draw in, and a grid that started at the screen's edge
/// would sit tighter under the bar than it does against the sides.
#[derive(Debug, Clone, Copy)]
pub struct Canvas {
    /// The whole output, output-local.
    pub output: Rectangle<i32, Logical>,
    /// What is left of it after layer surfaces took their exclusive zones.
    pub usable: Rectangle<i32, Logical>,
}

impl Canvas {
    pub fn new(output: Rectangle<i32, Logical>, usable: Rectangle<i32, Logical>) -> Self {
        Self { output, usable }
    }

    /// A canvas nothing has reserved space on.
    pub fn whole(output: Rectangle<i32, Logical>) -> Self {
        Self::new(output, output)
    }
}

/// Proportions of the output, so the overview looks the same on a laptop panel
/// and a 4K desktop instead of being tuned for one of them.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// Height of the workspace bar — thumbnails and their labels — as a
    /// fraction of the output height.
    pub bar: f64,
    /// Height of a workspace label as a fraction of the output height.
    pub label: f64,
    /// Breathing room around the grid and between thumbnails, as a fraction of
    /// the output's shorter side.
    pub gap: f64,
    /// How much a hovered thumbnail grows.
    pub hover: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            bar: 0.16,
            label: 0.022,
            gap: 0.022,
            hover: 0.04,
        }
    }
}

impl Metrics {
    fn gap_px(&self, output: Size<i32, Logical>) -> i32 {
        (f64::from(output.w.min(output.h)) * self.gap).round() as i32
    }

    /// How tall the workspace bar is, previews and labels together.
    fn bar_px(&self, output: Size<i32, Logical>) -> f64 {
        f64::from(output.h) * self.bar
    }
}

/// One workspace's place in the bottom bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// The preview box, in the output's own aspect ratio.
    pub thumb: Rectangle<f64, Logical>,
    /// Where the workspace's name goes, directly under its preview.
    pub label: Rectangle<f64, Logical>,
}

impl Slot {
    /// The whole slot, preview and label together — what a click or a drop
    /// tests against, so the label is as good a target as the picture.
    pub fn hit(&self) -> Rectangle<f64, Logical> {
        let bottom = self.label.loc.y + self.label.size.h;
        Rectangle::new(
            self.thumb.loc,
            (self.thumb.size.w, bottom - self.thumb.loc.y).into(),
        )
    }
}

/// The region the window grid is laid out in: the usable area, less the bar
/// and the same margin on every side — including the one under whatever
/// reserved the top, which is what stops the grid tucking in under a bar.
pub fn grid_area(canvas: Canvas, metrics: &Metrics) -> Rectangle<i32, Logical> {
    let gap = metrics.gap_px(canvas.output.size);
    let bar = metrics.bar_px(canvas.output.size).round() as i32;
    let usable = canvas.usable;

    let top = usable.loc.y + gap;
    let bottom = usable.loc.y + usable.size.h - bar - gap * 2;

    Rectangle::new(
        (usable.loc.x + gap, top).into(),
        (
            (usable.size.w - gap * 2).max(1),
            (bottom - top).max(1),
        )
            .into(),
    )
}

/// Places the active workspace's windows across [`grid_area`].
///
/// `windows` are the windows' real sizes in the order they should read, and
/// the results are index-aligned with them.
pub fn grid(
    canvas: Canvas,
    windows: &[Size<i32, Logical>],
    metrics: &Metrics,
    out: &mut Vec<Rectangle<f64, Logical>>,
) {
    layout::solve(
        grid_area(canvas, metrics),
        windows,
        metrics.gap_px(canvas.output.size),
        out,
    );
}

/// Places `count` workspace previews along the bottom edge.
///
/// Every preview has the output's own aspect ratio, so a workspace reads as a
/// small copy of the screen. They shrink to fit rather than scrolling: a
/// thumbnail you cannot see is not a target you can drop a window on.
pub fn bar(canvas: Canvas, count: usize, metrics: &Metrics, out: &mut Vec<Slot>) {
    out.clear();
    if count == 0 {
        return;
    }

    let output = canvas.output;
    let usable = canvas.usable;
    let gap = f64::from(metrics.gap_px(output.size));
    let label = f64::from(output.size.h) * metrics.label;
    let bar = metrics.bar_px(output.size);

    let aspect = f64::from(output.size.w) / f64::from(output.size.h.max(1));
    // The tallest a preview can be before the row is wider than the output.
    let widest = (f64::from(usable.size.w) - gap * (count + 1) as f64) / count as f64 / aspect;
    let height = (bar - label - gap).min(widest).max(1.0);
    let width = height * aspect;

    let spread = width * count as f64 + gap * (count - 1) as f64;
    let mut x = f64::from(usable.loc.x) + (f64::from(usable.size.w) - spread) / 2.0;
    let y = f64::from(usable.loc.y + usable.size.h) - gap - label - height;

    out.reserve(count);
    for _ in 0..count {
        out.push(Slot {
            thumb: Rectangle::new((x, y).into(), (width, height).into()),
            label: Rectangle::new((x, y + height).into(), (width, label).into()),
        });
        x += width + gap;
    }
}

/// How far the workspace bar still has to rise, given how far it has arrived.
///
/// The bar enters from below the bottom edge rather than fading in on the spot,
/// and both the previews and their labels have to agree on by how much.
pub fn climb(canvas: Canvas, metrics: &Metrics, bar: f64) -> f64 {
    metrics.bar_px(canvas.output.size) * (1.0 - bar)
}

/// Shrinks a window's real geometry into a workspace preview, so the preview is
/// a faithful small copy of that workspace.
pub fn inside(
    window: Rectangle<i32, Logical>,
    workspace: Rectangle<i32, Logical>,
    thumb: Rectangle<f64, Logical>,
) -> Rectangle<f64, Logical> {
    let scale = thumb.size.w / f64::from(workspace.size.w.max(1));
    Rectangle::new(
        (
            thumb.loc.x + f64::from(window.loc.x - workspace.loc.x) * scale,
            thumb.loc.y + f64::from(window.loc.y - workspace.loc.y) * scale,
        )
            .into(),
        (
            f64::from(window.size.w) * scale,
            f64::from(window.size.h) * scale,
        )
            .into(),
    )
}

/// Carries a rectangle from `from` to `to`. `t` is the overview's progress, so
/// a window flies to its thumbnail and back along the same path.
pub fn between(
    from: Rectangle<f64, Logical>,
    to: Rectangle<f64, Logical>,
    t: f64,
) -> Rectangle<f64, Logical> {
    let lerp = |a: f64, b: f64| a + (b - a) * t;
    Rectangle::new(
        (lerp(from.loc.x, to.loc.x), lerp(from.loc.y, to.loc.y)).into(),
        (lerp(from.size.w, to.size.w), lerp(from.size.h, to.size.h)).into(),
    )
}

/// Grows a rectangle about its own centre, for the lift under the pointer.
/// `amount` is a fraction of its size, so 0.04 is four percent bigger.
pub fn lift(rect: Rectangle<f64, Logical>, amount: f64) -> Rectangle<f64, Logical> {
    let (dw, dh) = (rect.size.w * amount, rect.size.h * amount);
    Rectangle::new(
        (rect.loc.x - dw / 2.0, rect.loc.y - dh / 2.0).into(),
        (rect.size.w + dw, rect.size.h + dh).into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), (1920, 1080).into())
    }

    fn canvas() -> Canvas {
        Canvas::whole(output())
    }

    fn slots(count: usize) -> Vec<Slot> {
        let mut out = Vec::new();
        bar(canvas(), count, &Metrics::default(), &mut out);
        out
    }

    #[test]
    fn the_grid_leaves_room_for_the_bar() {
        let metrics = Metrics::default();
        let area = grid_area(canvas(), &metrics);
        let lowest = slots(3)
            .iter()
            .map(|slot| slot.thumb.loc.y)
            .fold(f64::MAX, f64::min);

        assert!(
            f64::from(area.loc.y + area.size.h) <= lowest,
            "the grid runs into the bar"
        );
        assert!(area.size.h > 0 && area.size.w > 0);
    }

    #[test]
    fn the_grid_sits_inside_its_output() {
        let area = grid_area(canvas(), &Metrics::default());
        assert!(area.loc.x > 0 && area.loc.y > 0);
        assert!(area.loc.x + area.size.w <= 1920);
    }

    #[test]
    fn no_workspaces_means_no_bar() {
        assert!(slots(0).is_empty());
    }

    #[test]
    fn previews_carry_the_output_aspect_ratio() {
        for slot in slots(4) {
            let aspect = slot.thumb.size.w / slot.thumb.size.h;
            assert!((aspect - 16.0 / 9.0).abs() < 1e-6, "{aspect}");
        }
    }

    #[test]
    fn the_bar_is_centred_and_ordered() {
        let slots = slots(5);
        let left = slots[0].thumb.loc.x;
        let right = slots[4].thumb.loc.x + slots[4].thumb.size.w;
        assert!((left - (1920.0 - right)).abs() < 1e-6, "{left} {right}");

        for pair in slots.windows(2) {
            assert!(pair[1].thumb.loc.x > pair[0].thumb.loc.x);
        }
    }

    #[test]
    fn previews_never_overlap_and_stay_on_screen() {
        for count in 1..=24 {
            let slots = slots(count);
            assert_eq!(slots.len(), count);
            for slot in &slots {
                assert!(slot.thumb.loc.x >= -1e-6, "{slot:?}");
                assert!(
                    slot.thumb.loc.x + slot.thumb.size.w <= 1920.0 + 1e-6,
                    "{slot:?}"
                );
                assert!(
                    slot.label.loc.y + slot.label.size.h <= 1080.0 + 1e-6,
                    "{slot:?}"
                );
            }
            for pair in slots.windows(2) {
                assert!(
                    pair[0].thumb.loc.x + pair[0].thumb.size.w <= pair[1].thumb.loc.x + 1e-6,
                    "{pair:?}"
                );
            }
        }
    }

    #[test]
    fn a_crowded_bar_shrinks_instead_of_running_off() {
        let few = slots(3)[0].thumb.size.w;
        let many = slots(20)[0].thumb.size.w;
        assert!(many < few, "{many} against {few}");
    }

    #[test]
    fn the_label_sits_directly_under_its_preview() {
        for slot in slots(3) {
            assert!((slot.label.loc.y - (slot.thumb.loc.y + slot.thumb.size.h)).abs() < 1e-6);
            assert_eq!(slot.label.loc.x, slot.thumb.loc.x);
            assert_eq!(slot.label.size.w, slot.thumb.size.w);
        }
    }

    #[test]
    fn a_slots_hit_area_covers_both_its_parts() {
        let slot = slots(3)[1];
        let hit = slot.hit();
        assert_eq!(hit.loc, slot.thumb.loc);
        assert!(hit.size.h > slot.thumb.size.h);
        assert!((hit.loc.y + hit.size.h - (slot.label.loc.y + slot.label.size.h)).abs() < 1e-6);
    }

    #[test]
    fn an_offset_output_moves_the_whole_overview() {
        let mut shifted = Vec::new();
        bar(
            Canvas::whole(Rectangle::new((1920, 200).into(), (1920, 1080).into())),
            3,
            &Metrics::default(),
            &mut shifted,
        );
        for (a, b) in slots(3).iter().zip(&shifted) {
            assert!((b.thumb.loc.x - a.thumb.loc.x - 1920.0).abs() < 1e-6);
            assert!((b.thumb.loc.y - a.thumb.loc.y - 200.0).abs() < 1e-6);
        }
    }

    #[test]
    fn a_window_shrinks_into_a_preview_in_proportion() {
        let workspace = output();
        let thumb = Rectangle::new((100.0, 50.0).into(), (192.0, 108.0).into());

        // A window filling the whole workspace fills the whole preview.
        let full = inside(workspace, workspace, thumb);
        assert!((full.loc.x - thumb.loc.x).abs() < 1e-6);
        assert!((full.size.w - thumb.size.w).abs() < 1e-6);
        assert!((full.size.h - thumb.size.h).abs() < 1e-6);

        // And one in the middle stays in the middle.
        let middle = inside(
            Rectangle::new((960, 540).into(), (480, 270).into()),
            workspace,
            thumb,
        );
        assert!((middle.loc.x - (thumb.loc.x + thumb.size.w / 2.0)).abs() < 1e-6);
        assert!((middle.size.w - thumb.size.w / 4.0).abs() < 1e-6);
    }

    #[test]
    fn the_bar_climbs_its_own_height_and_no_further() {
        let metrics = Metrics::default();
        let full = climb(canvas(), &metrics, 0.0);

        assert!(full > 0.0, "a bar that has not arrived is below the edge");
        assert_eq!(
            climb(canvas(), &metrics, 1.0),
            0.0,
            "arrived means in place"
        );
        assert!((climb(canvas(), &metrics, 0.5) - full / 2.0).abs() < 1e-9);
        assert!(full <= f64::from(output().size.h) * metrics.bar + 1e-9);
    }

    #[test]
    fn between_lands_on_its_ends() {
        let from = Rectangle::new((0.0, 0.0).into(), (100.0, 100.0).into());
        let to = Rectangle::new((50.0, 20.0).into(), (10.0, 30.0).into());

        assert_eq!(between(from, to, 0.0), from);
        assert_eq!(between(from, to, 1.0), to);
    }

    #[test]
    fn between_moves_steadily_across() {
        let from = Rectangle::new((0.0, 0.0).into(), (100.0, 100.0).into());
        let to = Rectangle::new((100.0, 200.0).into(), (200.0, 50.0).into());
        let half = between(from, to, 0.5);

        assert_eq!(half.loc.x, 50.0);
        assert_eq!(half.loc.y, 100.0);
        assert_eq!(half.size.w, 150.0);
        assert_eq!(half.size.h, 75.0);
    }

    #[test]
    fn a_lift_grows_about_the_centre() {
        let rect = Rectangle::new((100.0, 100.0).into(), (200.0, 100.0).into());
        let lifted = lift(rect, 0.1);

        let centre =
            |r: Rectangle<f64, Logical>| (r.loc.x + r.size.w / 2.0, r.loc.y + r.size.h / 2.0);
        assert_eq!(centre(rect), centre(lifted));
        assert!((lifted.size.w - 220.0).abs() < 1e-6);
        assert!((lifted.size.h - 110.0).abs() < 1e-6);
    }

    #[test]
    fn the_grid_places_windows_where_the_solver_says() {
        let windows = [Size::from((1280, 720)), Size::from((800, 600))];
        let metrics = Metrics::default();

        let (mut scene, mut solved) = (Vec::new(), Vec::new());
        grid(canvas(), &windows, &metrics, &mut scene);
        layout::solve(
            grid_area(canvas(), &metrics),
            &windows,
            metrics.gap_px(output().size),
            &mut solved,
        );

        assert_eq!(scene, solved);
    }

    #[test]
    fn the_grid_clears_a_reserved_strip_by_the_same_margin_as_the_sides() {
        let metrics = Metrics::default();
        let bar = 48;
        let canvas = Canvas::new(
            output(),
            Rectangle::new((0, bar).into(), (1920, 1080 - bar).into()),
        );
        let area = grid_area(canvas, &metrics);

        assert_eq!(
            area.loc.y - bar,
            area.loc.x,
            "the top margin should match the side one"
        );
        assert!(area.size.h > 0);
    }

    #[test]
    fn the_bar_sits_above_a_reserved_bottom_strip() {
        let metrics = Metrics::default();
        let reserved = 60;
        let mut out = Vec::new();
        bar(
            Canvas::new(
                output(),
                Rectangle::new((0, 0).into(), (1920, 1080 - reserved).into()),
            ),
            3,
            &metrics,
            &mut out,
        );

        for slot in &out {
            assert!(
                slot.label.loc.y + slot.label.size.h <= f64::from(1080 - reserved) + 1e-6,
                "{slot:?} runs into the reserved strip"
            );
        }
    }
}

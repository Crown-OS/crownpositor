//! The shape a liquid morph draws: a rounded quadrilateral between the icon
//! and the window, worked out along the axis the window travels and across
//! it, so one model serves a dock on any edge.

use smithay::utils::{Logical, Point, Rectangle};

use super::{Edge, IconOrigin, lerp};

/// How far each channel of the morph has come, 0 on the icon and 1 on the
/// window. A spring that overshoots takes it past either end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub travel: f64,
    pub length: f64,
    pub breadth: f64,
}

/// The quadrilateral this frame, in output-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiquidShape {
    /// Where the window's top left, top right, bottom left and bottom right
    /// corners are drawn.
    pub corners: [Point<f64, Logical>; 4],
    /// Corner radius, measured along the shape's edges.
    pub radius: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

/// One end of the shape along the axis of travel: where it is, and the
/// stretch across the axis it spans there.
#[derive(Debug, Clone, Copy)]
struct End {
    along: f64,
    across: (f64, f64),
}

impl LiquidShape {
    /// The shape `progress` of the way from `icon` to `window`, whose corners
    /// are rounded by `window_radius`. `taper` is how much of the leading
    /// edge's lead its breadth takes on.
    pub fn between(
        icon: &IconOrigin,
        window: Rectangle<f64, Logical>,
        window_radius: f64,
        progress: Progress,
        taper: f64,
    ) -> Self {
        let axis = Axis::of(icon, window);
        let (icon_along, window_along) = (axis.along(icon.rect), axis.along(window));
        let (icon_across, window_across) = (axis.across(icon.rect), axis.across(window));

        let icon_middle = (icon_along.0 + icon_along.1) / 2.0;
        let leads_low =
            (icon_middle - window_along.0).abs() >= (window_along.1 - icon_middle).abs();
        let pick = |(low, high): (f64, f64)| if leads_low { low } else { high };

        let lead = lerp(pick(icon_along), pick(window_along), progress.travel);
        let length = lerp(span(icon_along), span(window_along), progress.length).max(1.0);
        let trail = if leads_low {
            lead + length
        } else {
            lead - length
        };
        let lead_breadth = progress.breadth + taper * (progress.travel - progress.breadth);
        let across = |t: f64| {
            let low = lerp(icon_across.0, window_across.0, t);
            (low, lerp(icon_across.1, window_across.1, t).max(low + 1.0))
        };
        let lead = End {
            along: lead,
            across: across(lead_breadth),
        };
        let trail = End {
            along: trail,
            across: across(progress.breadth),
        };
        let (low, high) = if leads_low {
            (lead, trail)
        } else {
            (trail, lead)
        };

        let roundness = lerp(
            roundness(icon.rect, icon.radius),
            roundness(window, window_radius),
            progress.breadth.clamp(0.0, 1.0),
        );
        let narrowest = span(low.across).min(span(high.across)).min(length);
        Self {
            corners: axis.corners(low, high),
            radius: roundness * narrowest / 2.0,
        }
    }
}

impl Axis {
    /// Along the dock's edge's normal when the icon sits in one, otherwise
    /// the way the window's centre is farther from the icon's.
    fn of(icon: &IconOrigin, window: Rectangle<f64, Logical>) -> Self {
        match icon.edge {
            Some(Edge::Top | Edge::Bottom) => Self::Vertical,
            Some(Edge::Left | Edge::Right) => Self::Horizontal,
            None => {
                let (icon, window) = (centre(icon.rect), centre(window));
                match (icon.x - window.x).abs() > (icon.y - window.y).abs() {
                    true => Self::Horizontal,
                    false => Self::Vertical,
                }
            }
        }
    }

    fn along(self, rect: Rectangle<f64, Logical>) -> (f64, f64) {
        match self {
            Self::Vertical => (rect.loc.y, rect.loc.y + rect.size.h),
            Self::Horizontal => (rect.loc.x, rect.loc.x + rect.size.w),
        }
    }

    fn across(self, rect: Rectangle<f64, Logical>) -> (f64, f64) {
        match self {
            Self::Vertical => (rect.loc.x, rect.loc.x + rect.size.w),
            Self::Horizontal => (rect.loc.y, rect.loc.y + rect.size.h),
        }
    }

    /// Top left, top right, bottom left, bottom right.
    fn corners(self, low: End, high: End) -> [Point<f64, Logical>; 4] {
        match self {
            Self::Vertical => [
                (low.across.0, low.along),
                (low.across.1, low.along),
                (high.across.0, high.along),
                (high.across.1, high.along),
            ],
            Self::Horizontal => [
                (low.along, low.across.0),
                (high.along, high.across.0),
                (low.along, low.across.1),
                (high.along, high.across.1),
            ],
        }
        .map(Point::from)
    }
}

fn span((low, high): (f64, f64)) -> f64 {
    high - low
}

fn centre(rect: Rectangle<f64, Logical>) -> Point<f64, Logical> {
    rect.loc + rect.size.to_point().downscale(2.0)
}

/// `radius` as a share of the largest radius `rect` can take: 1 is a pill.
fn roundness(rect: Rectangle<f64, Logical>, radius: f64) -> f64 {
    let largest = rect.size.w.min(rect.size.h) / 2.0;
    if largest <= 0.0 {
        return 1.0;
    }
    (radius / largest).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAPER: f64 = 0.15;

    impl Progress {
        const fn even(progress: f64) -> Self {
            Self {
                travel: progress,
                length: progress,
                breadth: progress,
            }
        }
    }

    impl LiquidShape {
        fn bounds(&self) -> Rectangle<f64, Logical> {
            let (mut min, mut max) = (self.corners[0], self.corners[0]);
            for corner in &self.corners[1..] {
                min = (min.x.min(corner.x), min.y.min(corner.y)).into();
                max = (max.x.max(corner.x), max.y.max(corner.y)).into();
            }
            Rectangle::from_extremities(min, max)
        }
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn dock_icon() -> IconOrigin {
        IconOrigin {
            rect: rect(900.0, 1000.0, 64.0, 64.0),
            radius: 14.0,
            edge: Some(Edge::Bottom),
            carrier: None,
        }
    }

    fn window() -> Rectangle<f64, Logical> {
        rect(400.0, 100.0, 1000.0, 700.0)
    }

    fn shape(progress: Progress) -> LiquidShape {
        LiquidShape::between(&dock_icon(), window(), 10.0, progress, TAPER)
    }

    fn width(shape: &LiquidShape, top: bool) -> f64 {
        let [tl, tr, bl, br] = shape.corners;
        match top {
            true => tr.x - tl.x,
            false => br.x - bl.x,
        }
    }

    #[test]
    fn it_starts_on_the_icon_and_ends_on_the_window() {
        let on_icon = shape(Progress::even(0.0));
        assert_eq!(on_icon.bounds(), dock_icon().rect);
        assert!((on_icon.radius - 14.0).abs() < 1e-9);

        let on_window = shape(Progress::even(1.0));
        assert_eq!(on_window.bounds(), window());
        assert!((on_window.radius - 10.0).abs() < 1e-9);
    }

    #[test]
    fn evenly_spread_progress_keeps_it_a_rectangle() {
        let [tl, tr, bl, br] = shape(Progress::even(0.4)).corners;
        assert_eq!((tl.y, tl.x), (tr.y, bl.x));
        assert_eq!((bl.y, tr.x), (br.y, br.x));
    }

    #[test]
    fn the_edge_farthest_from_the_dock_leads_and_widens_first() {
        let growing = shape(Progress {
            travel: 0.9,
            length: 0.4,
            breadth: 0.3,
        });
        let [tl, ..] = growing.corners;
        assert!((tl.y - lerp(1000.0, 100.0, 0.9)).abs() < 1e-9);
        assert!(width(&growing, true) > width(&growing, false));
    }

    #[test]
    fn shrinking_it_collapses_before_the_leading_edge_follows() {
        let shrinking = shape(Progress {
            travel: 0.95,
            length: 0.5,
            breadth: 0.4,
        });
        assert!(width(&shrinking, true) > width(&shrinking, false));
        let [tl, _, bl, _] = shrinking.corners;
        assert!(
            bl.y < window().loc.y + window().size.h,
            "the trailing edge is dragged up"
        );
        assert!(tl.y < bl.y);
    }

    #[test]
    fn a_dock_on_the_left_stretches_it_sideways() {
        let icon = IconOrigin {
            rect: rect(0.0, 500.0, 64.0, 64.0),
            edge: Some(Edge::Left),
            ..dock_icon()
        };
        let [tl, tr, bl, br] = LiquidShape::between(
            &icon,
            window(),
            10.0,
            Progress {
                travel: 0.9,
                length: 0.4,
                breadth: 0.3,
            },
            TAPER,
        )
        .corners;
        assert!(tr.x > 1200.0 && tl.x < tr.x, "the right edge leads");
        assert!(br.y - tr.y > bl.y - tl.y, "and is the wider end");
    }

    #[test]
    fn an_overshooting_leading_edge_reaches_past_the_window() {
        let overshot = shape(Progress {
            travel: 1.1,
            length: 1.0,
            breadth: 1.0,
        });
        assert!(overshot.bounds().loc.y < window().loc.y);
    }

    #[test]
    fn a_pill_shaped_icon_stays_round_while_it_is_small() {
        let icon = IconOrigin {
            radius: 32.0,
            ..dock_icon()
        };
        let small = LiquidShape::between(&icon, window(), 10.0, Progress::even(0.1), TAPER);
        let bounds = small.bounds();
        let pill = bounds.size.w.min(bounds.size.h) / 2.0;
        assert!(small.radius > 0.8 * pill, "{} of {pill}", small.radius);
    }
}

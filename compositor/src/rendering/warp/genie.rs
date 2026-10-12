//! The genie's frame of reference: the axis from the window to its icon, and
//! both rectangles measured along and across it.

use smithay::utils::{Physical, Rectangle};

use crate::{shaders::warp::GenieUniforms, shell::morph::Edge};

/// Which way the window pours, as a unit axis in output coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pour {
    Down,
    Up,
    Right,
    Left,
}

impl Pour {
    /// Towards the dock's edge when the icon sits in one. Otherwise towards
    /// the side the icon lies beyond, by the widest gap between the two
    /// rectangles; an icon over the window — a launcher grid — goes by where
    /// its centre sits, relative to the window's size.
    fn toward(
        window: Rectangle<f64, Physical>,
        icon: Rectangle<f64, Physical>,
        edge: Option<Edge>,
    ) -> Self {
        match edge {
            Some(Edge::Bottom) => return Self::Down,
            Some(Edge::Top) => return Self::Up,
            Some(Edge::Right) => return Self::Right,
            Some(Edge::Left) => return Self::Left,
            None => {}
        }
        let gaps = [
            (Self::Down, icon.loc.y - (window.loc.y + window.size.h)),
            (Self::Up, window.loc.y - (icon.loc.y + icon.size.h)),
            (Self::Right, icon.loc.x - (window.loc.x + window.size.w)),
            (Self::Left, window.loc.x - (icon.loc.x + icon.size.w)),
        ];
        if let Some((pour, _)) = gaps
            .into_iter()
            .filter(|(_, gap)| *gap > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
        {
            return pour;
        }

        let dx = centre(icon).0 - centre(window).0;
        let dy = centre(icon).1 - centre(window).1;
        match (dx / window.size.w.max(1.0)).abs() > (dy / window.size.h.max(1.0)).abs() {
            true if dx > 0.0 => Self::Right,
            true => Self::Left,
            false if dy < 0.0 => Self::Up,
            false => Self::Down,
        }
    }

    fn along(self) -> [f32; 2] {
        match self {
            Self::Down => [0.0, 1.0],
            Self::Up => [0.0, -1.0],
            Self::Right => [1.0, 0.0],
            Self::Left => [-1.0, 0.0],
        }
    }

    fn across(self) -> [f32; 2] {
        match self {
            Self::Down | Self::Up => [1.0, 0.0],
            Self::Right | Self::Left => [0.0, 1.0],
        }
    }
}

fn centre(rect: Rectangle<f64, Physical>) -> (f64, f64) {
    (
        rect.loc.x + rect.size.w / 2.0,
        rect.loc.y + rect.size.h / 2.0,
    )
}

/// `rect`'s extent across, then along, `pour`: each from low to high.
fn span(rect: Rectangle<f64, Physical>, pour: Pour) -> [f32; 4] {
    let x = (rect.loc.x as f32, (rect.loc.x + rect.size.w) as f32);
    let y = (rect.loc.y as f32, (rect.loc.y + rect.size.h) as f32);
    match pour {
        Pour::Down => [x.0, x.1, y.0, y.1],
        Pour::Up => [x.0, x.1, -y.1, -y.0],
        Pour::Right => [y.0, y.1, x.0, x.1],
        Pour::Left => [y.0, y.1, -x.1, -x.0],
    }
}

pub fn uniforms(
    window: Rectangle<f64, Physical>,
    icon: Rectangle<f64, Physical>,
    edge: Option<Edge>,
    progress: f64,
) -> GenieUniforms {
    let pour = Pour::toward(window, icon, edge);
    GenieUniforms {
        along: pour.along(),
        across: pour.across(),
        window_span: span(window, pour),
        icon_span: span(icon, pour),
        progress: progress as f32,
    }
}

/// Everywhere the genie can draw: it never leaves the hull of the two
/// rectangles.
pub fn bounds(
    window: Rectangle<f64, Physical>,
    icon: Rectangle<f64, Physical>,
) -> Rectangle<i32, Physical> {
    let hull = window.merge(icon);
    Rectangle::from_extremities(
        (hull.loc.x.floor() as i32, hull.loc.y.floor() as i32),
        (
            (hull.loc.x + hull.size.w).ceil() as i32,
            (hull.loc.y + hull.size.h).ceil() as i32,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn window() -> Rectangle<f64, Physical> {
        rect(400.0, 200.0, 1000.0, 600.0)
    }

    #[test]
    fn it_pours_towards_the_edge_the_icon_lies_beyond() {
        let cases = [
            (rect(500.0, 1000.0, 64.0, 64.0), Pour::Down),
            (rect(900.0, 20.0, 64.0, 64.0), Pour::Up),
            (rect(1700.0, 500.0, 64.0, 64.0), Pour::Right),
            (rect(10.0, 500.0, 64.0, 64.0), Pour::Left),
        ];
        for (icon, pour) in cases {
            assert_eq!(Pour::toward(window(), icon, None), pour, "{icon:?}");
        }
    }

    #[test]
    fn a_dock_icon_pours_towards_its_edge_wherever_the_window_is() {
        let far_left = rect(20.0, 1000.0, 64.0, 64.0);
        assert_eq!(Pour::toward(window(), far_left, None), Pour::Left);
        assert_eq!(
            Pour::toward(window(), far_left, Some(Edge::Bottom)),
            Pour::Down
        );
    }

    #[test]
    fn an_icon_over_the_window_goes_by_its_centre() {
        assert_eq!(
            Pour::toward(window(), rect(1250.0, 450.0, 64.0, 64.0), None),
            Pour::Right
        );
        assert_eq!(
            Pour::toward(window(), rect(870.0, 650.0, 64.0, 64.0), None),
            Pour::Down
        );
    }

    #[test]
    fn spans_run_from_the_far_edge_to_the_near_one() {
        for icon in [
            rect(100.0, 1000.0, 64.0, 64.0),
            rect(900.0, 20.0, 64.0, 64.0),
            rect(1700.0, 500.0, 64.0, 64.0),
            rect(10.0, 500.0, 64.0, 64.0),
        ] {
            let genie = uniforms(window(), icon, None, 0.5);
            let (window, icon) = (genie.window_span, genie.icon_span);
            assert!(window[0] < window[1] && window[2] < window[3]);
            assert!(icon[0] < icon[1] && icon[2] < icon[3]);
            assert!(icon[2] > window[3], "the icon must lie ahead: {genie:?}");
        }
    }

    #[test]
    fn the_bounds_hold_both_rectangles() {
        let icon = rect(100.5, 1000.25, 64.0, 64.0);
        let bounds = bounds(window(), icon).to_f64();
        assert!(bounds.contains_rect(window()));
        assert!(bounds.contains_rect(icon));
    }
}

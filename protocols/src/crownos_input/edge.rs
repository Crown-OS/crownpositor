//! The geometry `crownos_input_v1` depends on: which armed edge a motion went
//! through, and where a point given in an output's own space lands in the
//! layout.

use crownos_protocols::input::v1::server::crownos_input_capture_v1::Edge;
use smithay::utils::{Logical, Point, Rectangle};

/// A motion through an armed edge of the layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeCrossing {
    pub edge: Edge,
    /// Where along the edge, in the output's logical space: x for the top and
    /// bottom edges, y for the left and right ones.
    pub position: f64,
}

/// Which armed edge of `output` a motion from inside it to `to` crossed.
///
/// `None` when `to` is still on some output — the motion just moves the
/// pointer, possibly onto a neighbour — or when every edge it went through is
/// disarmed. A diagonal exit through a corner picks the armed edge the motion
/// overshot the most, which is the one the user was pushing against.
pub fn crossed_edge(
    output: Rectangle<i32, Logical>,
    layout: &[Rectangle<i32, Logical>],
    to: Point<f64, Logical>,
    armed: Edge,
) -> Option<EdgeCrossing> {
    if armed.is_empty() || layout.iter().any(|rect| contains(*rect, to)) {
        return None;
    }

    let left = f64::from(output.loc.x);
    let top = f64::from(output.loc.y);
    let right = left + f64::from(output.size.w);
    let bottom = top + f64::from(output.size.h);

    let along_x = (to.x.clamp(left, right - 1.0) - left).max(0.0);
    let along_y = (to.y.clamp(top, bottom - 1.0) - top).max(0.0);

    [
        (Edge::Left, left - to.x, along_y),
        (Edge::Right, to.x - (right - 1.0), along_y),
        (Edge::Top, top - to.y, along_x),
        (Edge::Bottom, to.y - (bottom - 1.0), along_x),
    ]
    .into_iter()
    .filter(|(edge, overshoot, _)| *overshoot > 0.0 && armed.contains(*edge))
    .max_by(|(_, first, _), (_, second, _)| first.total_cmp(second))
    .map(|(edge, _, position)| EdgeCrossing { edge, position })
}

/// A point in an output's logical space, as the layout sees it, clamped onto
/// the output. The far edge is exclusive, as it is for the pointer.
pub fn output_point_to_layout(
    output: Rectangle<i32, Logical>,
    point: Point<f64, Logical>,
) -> Point<f64, Logical> {
    let width = f64::from(output.size.w.max(1));
    let height = f64::from(output.size.h.max(1));
    Point::from((
        f64::from(output.loc.x) + point.x.clamp(0.0, width - 1.0),
        f64::from(output.loc.y) + point.y.clamp(0.0, height - 1.0),
    ))
}

fn contains(rect: Rectangle<i32, Logical>, point: Point<f64, Logical>) -> bool {
    let left = f64::from(rect.loc.x);
    let top = f64::from(rect.loc.y);
    point.x >= left
        && point.y >= top
        && point.x < left + f64::from(rect.size.w)
        && point.y < top + f64::from(rect.size.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    const LAPTOP: (i32, i32, i32, i32) = (0, 0, 1920, 1080);

    fn laptop() -> Rectangle<i32, Logical> {
        let (x, y, w, h) = LAPTOP;
        rect(x, y, w, h)
    }

    #[test]
    fn leaving_through_an_armed_edge_is_a_crossing() {
        let crossing = crossed_edge(laptop(), &[laptop()], (1925.0, 400.0).into(), Edge::Right);
        assert_eq!(
            crossing,
            Some(EdgeCrossing {
                edge: Edge::Right,
                position: 400.0
            })
        );
    }

    #[test]
    fn a_disarmed_edge_holds_the_pointer() {
        assert_eq!(
            crossed_edge(laptop(), &[laptop()], (-3.0, 400.0).into(), Edge::Right),
            None
        );
        assert_eq!(
            crossed_edge(laptop(), &[laptop()], (1925.0, 400.0).into(), Edge::empty()),
            None
        );
    }

    #[test]
    fn moving_onto_a_neighbour_is_not_a_crossing() {
        let monitor = rect(1920, 0, 2560, 1440);
        assert_eq!(
            crossed_edge(
                laptop(),
                &[laptop(), monitor],
                (1925.0, 400.0).into(),
                Edge::Right
            ),
            None
        );
    }

    #[test]
    fn an_l_shaped_layout_has_an_edge_where_the_neighbour_ends() {
        let short = rect(1920, 0, 1280, 720);
        let crossing = crossed_edge(
            laptop(),
            &[laptop(), short],
            (1925.0, 900.0).into(),
            Edge::Right,
        );
        assert_eq!(crossing.map(|crossing| crossing.edge), Some(Edge::Right));
    }

    #[test]
    fn a_corner_exit_takes_the_edge_pushed_hardest() {
        let armed = Edge::Top | Edge::Left;
        let crossing = crossed_edge(laptop(), &[laptop()], (-1.0, -6.0).into(), armed);
        assert_eq!(
            crossing,
            Some(EdgeCrossing {
                edge: Edge::Top,
                position: 0.0
            })
        );

        let only_left = crossed_edge(laptop(), &[laptop()], (-1.0, -6.0).into(), Edge::Left);
        assert_eq!(only_left.map(|crossing| crossing.edge), Some(Edge::Left));
    }

    #[test]
    fn the_crossing_position_is_output_local() {
        let right_monitor = rect(1920, 200, 1280, 1024);
        let crossing = crossed_edge(
            right_monitor,
            &[laptop(), right_monitor],
            (2500.0, 1300.0).into(),
            Edge::Bottom,
        );
        assert_eq!(
            crossing,
            Some(EdgeCrossing {
                edge: Edge::Bottom,
                position: 580.0
            })
        );
    }

    #[test]
    fn output_points_map_through_the_layout_and_clamp() {
        let monitor = rect(1920, 200, 1280, 1024);
        assert_eq!(
            output_point_to_layout(monitor, (10.0, 20.0).into()),
            Point::from((1930.0, 220.0))
        );
        assert_eq!(
            output_point_to_layout(monitor, (5000.0, -9.0).into()),
            Point::from((1920.0 + 1279.0, 200.0))
        );
    }
}

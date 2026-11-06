use tiny_skia::Point;

pub fn scaled(point: Point, by: f32) -> Point {
    Point::from_xy(point.x * by, point.y * by)
}

pub fn lerp(from: f32, to: f32, share: f32) -> f32 {
    from + (to - from) * share
}

/// The unit vector from `from` to `to`, or `None` where they meet.
pub fn direction(from: Point, to: Point) -> Option<Point> {
    let length = from.distance(to);
    (length > f32::EPSILON).then(|| scaled(to - from, 1.0 / length))
}

/// `direction` turned a quarter counter-clockwise, y up.
pub fn left_normal(direction: Point) -> Point {
    Point::from_xy(-direction.y, direction.x)
}

pub fn rotated(point: Point, angle: f32) -> Point {
    let (sin, cos) = angle.sin_cos();
    Point::from_xy(point.x * cos - point.y * sin, point.x * sin + point.y * cos)
}

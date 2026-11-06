//! The outline a pen leaves along a traced line: both sides offset by the
//! pen's reach, open ends cut off along their `Cut`. Every outline winds
//! clockwise, y up, and every counter the other way, so overlapping strokes
//! fill as one under the non-zero rule.

use tiny_skia::{PathBuilder, Point};

use super::frame::Station;
use super::skeleton::{Cut, Line};
use crate::util::vector::{direction, left_normal, lerp, scaled};

/// Of the pen's reach: how far a sharp corner's point may run before it is
/// bevelled.
const MITER_LIMIT: f32 = 4.0;
/// Below this cross product two runs are taken as one straight run.
const PARALLEL: f32 = 1e-4;
/// Logical pixels below which neighbouring stations are taken as one.
const MIN_GAP: f32 = 1e-3;
/// Logical pixels a colon dot's outline strays from its circle at most.
const DOT_TOLERANCE: f32 = 0.02;

/// An oval nib held level: a stroke drawn straight down is `stem` wide, one
/// drawn level is `hairline` tall.
#[derive(Debug, Clone, Copy)]
pub(super) struct Pen {
    pub stem: f32,
    pub hairline: f32,
}

impl Pen {
    /// The point of the nib's rim farthest along `normal`, from its centre,
    /// with `heft` of its stem.
    fn reach(self, normal: Point, heft: f32) -> Point {
        let across = lerp(self.hairline, self.stem, heft) / 2.0;
        let up = self.hairline / 2.0;
        let rim = Point::from_xy(across * across * normal.x, up * up * normal.y);
        let extent = (rim.x * normal.x + rim.y * normal.y).sqrt();
        if extent > f32::EPSILON {
            scaled(rim, 1.0 / extent)
        } else {
            Point::zero()
        }
    }
}

/// Adds the outline `pen` leaves along `stations`, traced from `line`.
pub(super) fn push(path: &mut PathBuilder, stations: &[Station], line: &Line, pen: Pen) {
    let stations = distinct(stations, line.closed);
    if stations.len() < 2 {
        return;
    }
    let left = side(&stations, pen, 1.0, line.closed);
    let right = side(&stations, pen, -1.0, line.closed);
    if line.closed {
        push_loop(path, left.into_iter());
        push_loop(path, right.into_iter().rev());
        return;
    }
    let [mut left, mut right] = [left, right];
    cap(&mut left, &mut right, &stations, line.start, line.end);
    push_loop(path, left.into_iter().chain(right.into_iter().rev()));
}

/// Adds a round dot, wound like a stroke's outline.
pub(super) fn push_dot(path: &mut PathBuilder, centre: Point, radius: f32) {
    let step = (8.0 * DOT_TOLERANCE / radius.max(DOT_TOLERANCE)).sqrt();
    let steps = (std::f32::consts::TAU / step).ceil().max(8.0) as usize;
    push_loop(
        path,
        (0..steps).map(|index| {
            let angle = -std::f32::consts::TAU * index as f32 / steps as f32;
            centre + Point::from_xy(radius * angle.cos(), radius * angle.sin())
        }),
    );
}

fn distinct(stations: &[Station], closed: bool) -> Vec<Station> {
    let mut kept: Vec<Station> = Vec::with_capacity(stations.len());
    for station in stations {
        if kept
            .last()
            .is_none_or(|last| last.point.distance(station.point) > MIN_GAP)
        {
            kept.push(*station);
        }
    }
    if closed && kept.len() > 2 && kept[0].point.distance(kept[kept.len() - 1].point) <= MIN_GAP {
        kept.pop();
    }
    kept
}

/// One side of the outline, `sign` 1 for the left of the line's travel.
fn side(stations: &[Station], pen: Pen, sign: f32, closed: bool) -> Vec<Point> {
    let count = stations.len();
    let runs: Vec<Point> = (0..count)
        .map(|index| {
            direction(stations[index].point, stations[(index + 1) % count].point)
                .unwrap_or(Point::from_xy(1.0, 0.0))
        })
        .collect();
    let mut points = Vec::with_capacity(count + count / 4);
    for (index, station) in stations.iter().enumerate() {
        let before = (closed || index > 0).then(|| runs[(index + count - 1) % count]);
        let after = (closed || index + 1 < count).then(|| runs[index]);
        let offset =
            |run: Point| station.point + pen.reach(scaled(left_normal(run), sign), station.heft);
        match (before, after) {
            (Some(before), Some(after)) => join(&mut points, station, before, after, offset),
            (Some(run), None) | (None, Some(run)) => points.push(offset(run)),
            (None, None) => {}
        }
    }
    points
}

/// The side's corner where a run along `before` turns along `after`:
/// where the two offset runs meet, bevelled if that lies too far out.
fn join(
    points: &mut Vec<Point>,
    station: &Station,
    before: Point,
    after: Point,
    offset: impl Fn(Point) -> Point,
) {
    let (first, second) = (offset(before), offset(after));
    let turn = before.cross(after);
    if turn.abs() < PARALLEL {
        points.push(scaled(first + second, 0.5));
        return;
    }
    let miter = first + scaled(before, (second - first).cross(after) / turn);
    let limit = MITER_LIMIT
        * first
            .distance(station.point)
            .max(second.distance(station.point));
    if miter.distance(station.point) > limit {
        points.extend([first, second]);
    } else {
        points.push(miter);
    }
}

fn cap(left: &mut Vec<Point>, right: &mut Vec<Point>, stations: &[Station], start: Cut, end: Cut) {
    let count = stations.len();
    let (first, last) = (stations[0].point, stations[count - 1].point);
    if let Some(outward) = direction(stations[count - 2].point, last) {
        let across = cut_normal(end, outward);
        clip_tail(left, last, across);
        clip_tail(right, last, across);
    }
    if let Some(outward) = direction(stations[1].point, first) {
        let across = cut_normal(start, outward);
        for side in [&mut *left, &mut *right] {
            side.reverse();
            clip_tail(side, first, across);
            side.reverse();
        }
    }
}

/// The normal of the line an end is cut along, pointing away from the stroke.
fn cut_normal(cut: Cut, outward: Point) -> Point {
    match cut {
        Cut::Square => outward,
        Cut::Slant(degrees) => {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let normal = left_normal(Point::from_xy(cos, sin));
            if normal.dot(outward) < 0.0 {
                -normal
            } else {
                normal
            }
        }
    }
}

/// Trims or stretches the end of `side` to meet the line through `origin`
/// square to `across`.
fn clip_tail(side: &mut Vec<Point>, origin: Point, across: Point) {
    let beyond = |point: Point| (point - origin).dot(across);
    while side.len() > 2 && beyond(side[side.len() - 2]) > 0.0 {
        side.pop();
    }
    let [.., inner, outer] = side[..] else {
        return;
    };
    let (from, to) = (beyond(inner), beyond(outer));
    if (from - to).abs() > f32::EPSILON
        && let Some(end) = side.last_mut()
    {
        *end = inner + scaled(outer - inner, from / (from - to));
    }
}

fn push_loop(path: &mut PathBuilder, mut points: impl Iterator<Item = Point>) {
    let Some(first) = points.next() else {
        return;
    };
    path.move_to(first.x, first.y);
    for point in points {
        path.line_to(point.x, point.y);
    }
    path.close();
}

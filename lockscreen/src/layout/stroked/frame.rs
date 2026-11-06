use tiny_skia::Point;

use super::skeleton::{Line, Y};
use crate::util::vector::{direction, left_normal, lerp, rotated, scaled};

/// Logical pixels a flattened turn strays from its true curve at most.
const TOLERANCE: f32 = 0.04;
/// Radians below which a joint is taken for a straight run.
const MIN_TURN: f32 = 1e-3;

/// A point on a line's centre, and how much of the pen's stem it keeps.
#[derive(Debug, Clone, Copy)]
pub(super) struct Station {
    pub point: Point,
    pub heft: f32,
}

/// The box a skeleton is traced in: logical pixels, y up from the baseline.
pub(super) struct Frame {
    pub width: f32,
    /// The whole glyph's, stroke included.
    pub height: f32,
    /// Where the skeleton's left edge sits.
    pub left: f32,
    /// Where the skeleton's foot and top sit: half a level stroke inside the ink.
    pub foot: f32,
    pub top: f32,
    /// How many times taller than wide a turn is.
    pub aspect: f32,
}

impl Frame {
    pub fn y(&self, y: Y) -> f32 {
        match y {
            Y::Foot(widths) => self.foot + widths * self.width,
            Y::Top(widths) => self.top - widths * self.width,
            Y::Share(share) => lerp(self.foot, self.top, share),
            Y::Split(share, widths) => lerp(self.foot, self.top, share) + widths * self.width,
            Y::Brim => self.height,
            Y::Base => 0.0,
        }
    }

    /// `line`'s centre, its round joints turned on their radii.
    pub fn trace(&self, line: &Line) -> Vec<Station> {
        let points: Vec<Point> = line
            .joints
            .iter()
            .map(|joint| {
                Point::from_xy(
                    self.left + joint.x * self.width,
                    self.y(joint.y) / self.aspect,
                )
            })
            .collect();
        let count = points.len();
        let mut stations = Vec::with_capacity(count * 16);
        if count < 2 {
            return stations;
        }
        let ends = |index: usize| Station {
            point: points[index],
            heft: line.joints[index].heft,
        };
        if !line.closed {
            stations.push(ends(0));
        }
        let corners = if line.closed { 0..count } else { 1..count - 1 };
        for index in corners {
            self.turn(line, &points, index, &mut stations);
        }
        if !line.closed {
            stations.push(ends(count - 1));
        }
        for station in &mut stations {
            station.point.y *= self.aspect;
        }
        stations
    }

    /// The stations through joint `index`, its turn rounded as far as the
    /// runs either side leave room for.
    fn turn(&self, line: &Line, points: &[Point], index: usize, stations: &mut Vec<Station>) {
        let count = points.len();
        let (before, after) = ((index + count - 1) % count, (index + 1) % count);
        let joint = line.joints[index];
        let point = points[index];
        let sharp = Station {
            point,
            heft: joint.heft,
        };
        let (Some(incoming), Some(outgoing)) = (
            direction(points[before], point),
            direction(point, points[after]),
        ) else {
            stations.push(sharp);
            return;
        };
        let turn = incoming.dot(outgoing).clamp(-1.0, 1.0).acos();
        let half_turn_tan = (turn / 2.0).tan();
        if joint.radius <= 0.0 || turn < MIN_TURN || !half_turn_tan.is_finite() {
            stations.push(sharp);
            return;
        }
        let rounds =
            |at: usize| (line.closed || (at > 0 && at + 1 < count)) && line.joints[at].radius > 0.0;
        let reach =
            |other: usize| points[other].distance(point) * if rounds(other) { 0.5 } else { 1.0 };
        let room = reach(before).min(reach(after));
        let along = (joint.radius * self.width * half_turn_tan).min(room);
        let radius = along / half_turn_tan;
        let side = if incoming.cross(outgoing) > 0.0 {
            1.0
        } else {
            -1.0
        };
        let enter = point - scaled(incoming, along);
        let centre = enter + scaled(left_normal(incoming), side * radius);
        let heft_in = lerp(
            line.joints[before].heft,
            joint.heft,
            1.0 - along / points[before].distance(point),
        );
        let heft_out = lerp(
            joint.heft,
            line.joints[after].heft,
            along / point.distance(points[after]),
        );
        let steps = (turn / self.step(radius)).ceil().max(1.0) as usize;
        let spoke = enter - centre;
        stations.extend((0..=steps).map(|step| {
            let share = step as f32 / steps as f32;
            let heft = if share < 0.5 {
                lerp(heft_in, joint.heft, 2.0 * share)
            } else {
                lerp(joint.heft, heft_out, 2.0 * share - 1.0)
            };
            Station {
                point: centre + rotated(spoke, side * turn * share),
                heft,
            }
        }));
    }

    /// Radians a turn on `radius` may advance per flattened step.
    fn step(&self, radius: f32) -> f32 {
        let longest = radius * self.aspect.max(1.0);
        (8.0 * TOLERANCE / longest.max(TOLERANCE)).sqrt()
    }
}

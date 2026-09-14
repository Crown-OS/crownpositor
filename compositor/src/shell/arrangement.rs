//! Where monitors sit relative to one another, and how focus moves between
//! them.
//!
//! This used to be a left-to-right pack at `y = 0`, which made both questions
//! index arithmetic: the monitor to the right was the next one in the list.
//! Once a GUI can drag a monitor anywhere — above, below, or diagonally — the
//! list order stops describing the geometry, so placement normalises the
//! layout and traversal asks the rectangles instead.

use smithay::utils::{Logical, Point, Rectangle};

use crate::{layout::Direction, shell::Shell, shell::monitor::Monitor};

impl Shell {
    /// Places every monitor and re-establishes the ordering invariants.
    ///
    /// Config-pinned outputs keep their position; the rest are packed to the
    /// right of everything already placed. The result is then shifted so the
    /// union's top-left corner is the origin — dragging a monitor to the left
    /// of the first one produces negative coordinates, and neither clients nor
    /// the pointer have any use for an origin that wanders.
    pub fn arrange_outputs(&mut self) {
        for monitor in &mut self.monitors {
            if let Some(fixed) = monitor.fixed_position() {
                monitor.set_position(fixed);
            }
        }

        // Start packing past the rightmost pinned output, so an auto-placed
        // output cannot land on top of one the user placed deliberately.
        let mut x = self
            .monitors
            .iter()
            .filter(|monitor| monitor.fixed_position().is_some())
            .map(|monitor| monitor.geometry().loc.x + monitor.geometry().size.w)
            .max()
            .unwrap_or(0);

        for monitor in &mut self.monitors {
            if monitor.fixed_position().is_some() {
                continue;
            }
            monitor.set_position((x, 0).into());
            x += monitor.config().logical_size().w;
        }

        self.normalize_positions();
        self.sort_monitors();
    }

    /// Shifts the whole layout so the union of every output starts at `(0, 0)`.
    ///
    /// Idempotent, and deliberately does not touch `fixed_position`: the pin
    /// is re-applied and re-normalised on every arrange, which lands in the
    /// same place, so rewriting it would only add a second source of truth.
    fn normalize_positions(&mut self) {
        let Some(origin) = self
            .monitors
            .iter()
            .map(|monitor| monitor.geometry().loc)
            .reduce(|acc, loc| Point::from((acc.x.min(loc.x), acc.y.min(loc.y))))
            .filter(|origin| origin.x != 0 || origin.y != 0)
        else {
            return;
        };

        for monitor in &mut self.monitors {
            let moved = monitor.config().position - origin;
            monitor.set_position(moved);
        }
    }

    /// Orders the list top-to-bottom then left-to-right, carrying focus.
    ///
    /// `focused_output` is a position in this list, so a sort that does not
    /// rebase it silently moves focus to whichever monitor lands there.
    fn sort_monitors(&mut self) {
        let focused = self.monitors.get(self.focused_output).map(Monitor::id);

        self.monitors.sort_by_key(|monitor| {
            let position = monitor.config().position;
            (position.y, position.x)
        });

        if let Some(focused) = focused {
            self.focused_output = self
                .monitors
                .iter()
                .position(|monitor| monitor.id() == focused)
                .unwrap_or(0);
        }
    }

    /// The monitor next to `from` in `dir`, by geometry.
    pub fn monitor_in_direction(&self, from: usize, dir: Direction) -> Option<usize> {
        let source = self.monitors.get(from)?.geometry();
        let candidates = self
            .monitors
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != from)
            .map(|(index, monitor)| (index, monitor.geometry()));

        nearest_in_direction(source, candidates, dir)
    }
}

/// Picks the neighbour a directional focus move should land on.
///
/// Monitors whose perpendicular span overlaps the source are strongly
/// preferred — that is what makes "the monitor to the right" mean the one
/// beside it rather than one three rows down. Among those the smallest gap
/// wins, ties broken by the largest overlap. Only when nothing overlaps does
/// the search widen to the nearest centre in that half-plane, so a purely
/// diagonal arrangement is still traversable instead of being a dead end.
fn nearest_in_direction(
    source: Rectangle<i32, Logical>,
    candidates: impl Iterator<Item = (usize, Rectangle<i32, Logical>)>,
    dir: Direction,
) -> Option<usize> {
    let mut overlapping: Option<(i32, i32, usize)> = None;
    let mut diagonal: Option<(i64, usize)> = None;

    for (index, candidate) in candidates {
        let Some(gap) = gap_towards(source, candidate, dir) else {
            continue;
        };
        let overlap = perpendicular_overlap(source, candidate, dir);

        if overlap > 0 {
            let key = (gap, -overlap, index);
            if overlapping.is_none_or(|best| key < best) {
                overlapping = Some(key);
            }
        } else {
            let distance = centre_distance_squared(source, candidate);
            if diagonal.is_none_or(|(best, _)| distance < best) {
                diagonal = Some((distance, index));
            }
        }
    }

    overlapping
        .map(|(_, _, index)| index)
        .or(diagonal.map(|(_, index)| index))
}

/// The distance from `source`'s edge to `candidate`'s facing edge, or `None`
/// when `candidate` is not strictly beyond that edge.
fn gap_towards(
    source: Rectangle<i32, Logical>,
    candidate: Rectangle<i32, Logical>,
    dir: Direction,
) -> Option<i32> {
    let gap = match dir {
        Direction::Left => source.loc.x - (candidate.loc.x + candidate.size.w),
        Direction::Right => candidate.loc.x - (source.loc.x + source.size.w),
        Direction::Up => source.loc.y - (candidate.loc.y + candidate.size.h),
        Direction::Down => candidate.loc.y - (source.loc.y + source.size.h),
    };
    (gap >= 0).then_some(gap)
}

/// How much the two rectangles share along the axis `dir` does not travel.
fn perpendicular_overlap(
    source: Rectangle<i32, Logical>,
    candidate: Rectangle<i32, Logical>,
    dir: Direction,
) -> i32 {
    let (source_start, source_end, candidate_start, candidate_end) = match dir {
        Direction::Left | Direction::Right => (
            source.loc.y,
            source.loc.y + source.size.h,
            candidate.loc.y,
            candidate.loc.y + candidate.size.h,
        ),
        Direction::Up | Direction::Down => (
            source.loc.x,
            source.loc.x + source.size.w,
            candidate.loc.x,
            candidate.loc.x + candidate.size.w,
        ),
    };

    (source_end.min(candidate_end) - source_start.max(candidate_start)).max(0)
}

/// Squared so the comparison needs no square root; `i64` because a 4-monitor
/// desktop is wide enough for `i32` to overflow when squared.
fn centre_distance_squared(
    source: Rectangle<i32, Logical>,
    candidate: Rectangle<i32, Logical>,
) -> i64 {
    let centre = |rectangle: Rectangle<i32, Logical>| {
        (
            rectangle.loc.x as i64 * 2 + rectangle.size.w as i64,
            rectangle.loc.y as i64 * 2 + rectangle.size.h as i64,
        )
    };
    let (source_x, source_y) = centre(source);
    let (candidate_x, candidate_y) = centre(candidate);
    let dx = candidate_x - source_x;
    let dy = candidate_y - source_y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// A laptop panel with an external monitor above and one to the right,
    /// which is the arrangement index arithmetic could never describe.
    fn l_shape() -> [Rectangle<i32, Logical>; 3] {
        [
            rect(0, 1080, 1920, 1080),
            rect(0, 0, 1920, 1080),
            rect(1920, 1080, 2560, 1440),
        ]
    }

    fn nearest(from: usize, dir: Direction) -> Option<usize> {
        let layout = l_shape();
        let candidates = layout
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, _)| *index != from);
        nearest_in_direction(layout[from], candidates, dir)
    }

    #[test]
    fn up_and_down_are_not_aliases_for_left_and_right() {
        assert_eq!(nearest(0, Direction::Up), Some(1));
        assert_eq!(nearest(1, Direction::Down), Some(0));
    }

    #[test]
    fn right_prefers_the_monitor_beside_rather_than_above() {
        assert_eq!(nearest(0, Direction::Right), Some(2));
    }

    #[test]
    fn an_edge_of_the_layout_has_no_neighbour() {
        assert_eq!(nearest(1, Direction::Up), None);
        assert_eq!(nearest(0, Direction::Left), None);
    }

    #[test]
    fn a_diagonal_neighbour_is_reachable_when_nothing_overlaps() {
        // The only candidate to the right is entirely above the source's span.
        let source = rect(0, 1080, 1920, 1080);
        let candidates = [(1usize, rect(1920, 0, 1920, 1080))];
        assert_eq!(
            nearest_in_direction(source, candidates.into_iter(), Direction::Right),
            Some(1)
        );
    }

    #[test]
    fn an_overlapping_neighbour_beats_a_closer_diagonal_one() {
        let source = rect(0, 0, 1920, 1080);
        let candidates = [
            // Diagonal and very close.
            (1usize, rect(1930, 2000, 800, 600)),
            // Overlapping but further away.
            (2usize, rect(2400, 0, 1920, 1080)),
        ];
        assert_eq!(
            nearest_in_direction(source, candidates.into_iter(), Direction::Right),
            Some(2)
        );
    }

    #[test]
    fn the_smallest_gap_wins_among_overlapping_neighbours() {
        let source = rect(0, 0, 1920, 1080);
        let candidates = [
            (1usize, rect(4000, 0, 1920, 1080)),
            (2usize, rect(1920, 0, 1920, 1080)),
        ];
        assert_eq!(
            nearest_in_direction(source, candidates.into_iter(), Direction::Right),
            Some(2)
        );
    }

    #[test]
    fn a_touching_neighbour_overlaps_by_its_whole_shared_edge() {
        let source = rect(0, 0, 1920, 1080);
        let beside = rect(1920, 0, 1920, 1080);
        assert_eq!(
            perpendicular_overlap(source, beside, Direction::Right),
            1080
        );
        assert_eq!(gap_towards(source, beside, Direction::Right), Some(0));
    }

    #[test]
    fn a_monitor_behind_the_source_is_not_a_candidate() {
        let source = rect(1920, 0, 1920, 1080);
        let behind = rect(0, 0, 1920, 1080);
        assert_eq!(gap_towards(source, behind, Direction::Right), None);
        assert_eq!(gap_towards(source, behind, Direction::Left), Some(0));
    }
}

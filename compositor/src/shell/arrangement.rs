//! Where monitors sit relative to one another, and how focus moves between
//! them.
//!
//! This used to be a left-to-right pack at `y = 0`, which made both questions
//! index arithmetic: the monitor to the right was the next one in the list.
//! Once a GUI can drag a monitor anywhere — above, below, or diagonally — the
//! list order stops describing the geometry, so placement normalises the
//! layout and traversal asks the rectangles instead.

use smithay::utils::{Logical, Point, Rectangle};

use crate::{layout::Direction, shell::Shell, shell::monitor::Monitor, utils::id::OutputId};

const HIDDEN_GAP: i32 = 1024;

impl Shell {
    /// Places every monitor and re-establishes the ordering invariants.
    ///
    /// Config-pinned outputs keep their position; the rest are packed to the
    /// right of everything already placed. The result is then shifted so the
    /// union's top-left corner is the origin — dragging a monitor to the left
    /// of the first one produces negative coordinates, and neither clients nor
    /// the pointer have any use for an origin that wanders.
    pub fn arrange_outputs(&mut self) {
        let pins: Vec<Option<Rectangle<i32, Logical>>> = self
            .monitors
            .iter()
            .map(|monitor| {
                monitor
                    .fixed_position()
                    .filter(|_| !monitor.is_hidden())
                    .map(|position| Rectangle::new(position, monitor.config().logical_size()))
            })
            .collect();
        let honoured = honoured_pins(&pins);

        for ((monitor, pin), honoured) in self.monitors.iter_mut().zip(&pins).zip(&honoured) {
            match (pin, honoured) {
                (Some(pin), true) => {
                    monitor.set_position(pin.loc);
                }
                (Some(pin), false) => tracing::warn!(
                    output = monitor.config().name,
                    position = ?pin.loc,
                    "the pinned position overlaps another monitor; packing it instead"
                ),
                (None, _) => {}
            }
        }

        // Start packing past the rightmost pinned output, so an auto-placed
        // output cannot land on top of one the user placed deliberately.
        let mut x = pins
            .iter()
            .zip(&honoured)
            .filter_map(|(pin, honoured)| pin.filter(|_| *honoured))
            .map(|pin| pin.loc.x + pin.size.w)
            .max()
            .unwrap_or(0);

        for (monitor, honoured) in self.monitors.iter_mut().zip(&honoured) {
            if *honoured || monitor.is_hidden() {
                continue;
            }
            monitor.set_position((x, 0).into());
            x += monitor.config().logical_size().w;
        }

        self.normalize_positions();
        self.park_hidden();
        self.monitors.sort_by_key(|monitor| {
            let position = monitor.config().position;
            (position.y, position.x)
        });
    }

    /// Hidden monitors go past the right edge with a gap, so no surface
    /// straddles one of them and a shown one, and they never move the origin.
    fn park_hidden(&mut self) {
        let mut x = self
            .shown_monitors()
            .map(|monitor| monitor.geometry().loc.x + monitor.geometry().size.w)
            .max()
            .unwrap_or(0)
            + HIDDEN_GAP;
        for monitor in self
            .monitors
            .iter_mut()
            .filter(|monitor| monitor.is_hidden())
        {
            monitor.set_position((x, 0).into());
            x += monitor.config().logical_size().w + HIDDEN_GAP;
        }
    }

    /// Shifts the whole layout so the union of every output starts at `(0, 0)`.
    ///
    /// Idempotent, and deliberately does not touch `fixed_position`: the pin
    /// is re-applied and re-normalised on every arrange, which lands in the
    /// same place, so rewriting it would only add a second source of truth.
    fn normalize_positions(&mut self) {
        let Some(origin) = self
            .shown_monitors()
            .map(|monitor| monitor.geometry().loc)
            .reduce(|acc, loc| Point::from((acc.x.min(loc.x), acc.y.min(loc.y))))
            .filter(|origin| origin.x != 0 || origin.y != 0)
        else {
            return;
        };

        for monitor in self
            .monitors
            .iter_mut()
            .filter(|monitor| !monitor.is_hidden())
        {
            let moved = monitor.config().position - origin;
            monitor.set_position(moved);
        }
    }

    /// The shown monitor next to `from` in `dir`, by geometry.
    pub fn monitor_in_direction(&self, from: OutputId, dir: Direction) -> Option<OutputId> {
        let source = self.monitor_by_id(from)?.geometry();
        let candidates: Vec<&Monitor> = self
            .shown_monitors()
            .filter(|monitor| monitor.id() != from)
            .collect();
        let nearest = nearest_in_direction(
            source,
            candidates
                .iter()
                .enumerate()
                .map(|(index, monitor)| (index, monitor.geometry())),
            dir,
        )?;
        candidates.get(nearest).map(|monitor| monitor.id())
    }
}

/// Which pins hold, in order: a pin that overlaps an earlier one that holds
/// is packed like an unpinned monitor instead.
pub fn honoured_pins(pins: &[Option<Rectangle<i32, Logical>>]) -> Vec<bool> {
    let mut held: Vec<Rectangle<i32, Logical>> = Vec::new();
    pins.iter()
        .map(|pin| {
            let Some(pin) = pin else {
                return false;
            };
            let holds = held.iter().all(|other| pin.intersection(*other).is_none());
            if holds {
                held.push(*pin);
            }
            holds
        })
        .collect()
}

/// The candidate whose centre is nearest to `from`'s.
pub fn nearest_by_centre(
    from: Rectangle<i32, Logical>,
    candidates: impl Iterator<Item = (OutputId, Rectangle<i32, Logical>)>,
) -> Option<OutputId> {
    candidates
        .min_by_key(|(_, candidate)| centre_distance_squared(from, *candidate))
        .map(|(id, _)| id)
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
    fn a_pin_overlapping_an_earlier_one_is_dropped_and_the_rest_hold() {
        let pins = [
            Some(rect(0, 0, 1920, 1080)),
            None,
            Some(rect(1000, 0, 1920, 1080)),
            Some(rect(1920, 0, 1280, 720)),
        ];
        assert_eq!(honoured_pins(&pins), [true, false, false, true]);
    }

    #[test]
    fn the_nearest_monitor_is_measured_centre_to_centre() {
        let [left, right] = [1, 3].map(OutputId::from_raw);
        let departing = rect(1920, 0, 1920, 1080);
        let candidates = [
            (left, rect(0, 0, 1920, 1080)),
            (right, rect(3840, 0, 1280, 720)),
        ];
        assert_eq!(
            nearest_by_centre(departing, candidates.into_iter()),
            Some(right)
        );
        assert_eq!(nearest_by_centre(departing, std::iter::empty()), None);
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

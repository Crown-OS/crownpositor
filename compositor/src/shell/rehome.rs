//! Moving a window's surfaces and geometry onto another monitor.
//!
//! Every path that changes a window's monitor ends here: a tile moved on its
//! own and a whole workspace adopted by another monitor alike.

use smithay::{
    output::Output,
    utils::{Logical, Rectangle},
};

use crate::{
    layout::placement,
    shell::{Location, Shell, monitor::Presence, workspace::Workspace},
    utils::id::WindowId,
};

/// Where a window comes from: the output it leaves, when it still has to
/// leave one, the usable area its floating rect was measured in, and whether
/// that monitor was shown.
pub(super) struct Origin {
    pub output: Option<Output>,
    pub area: Rectangle<i32, Logical>,
    pub presence: Presence,
}

/// The monitor a whole workspace leaves: its output, when the windows still
/// have to leave it, and whether it was shown.
pub(super) struct Departure {
    pub output: Option<Output>,
    pub presence: Presence,
}

impl Departure {
    /// A workspace whose windows already left their output, from a shown
    /// monitor or the stash.
    pub(super) const DETACHED: Self = Self {
        output: None,
        presence: Presence::Shown,
    };

    pub(super) fn with_area(self, area: Rectangle<i32, Logical>) -> Origin {
        Origin {
            output: self.output,
            area,
            presence: self.presence,
        }
    }
}

/// One end of a move: whether the monitor is shown, and its usable area.
#[derive(Debug, Clone, Copy)]
struct End {
    presence: Presence,
    area: Rectangle<i32, Logical>,
}

/// The floating rect after a move from `from` to `to`. It is carried only
/// between two shown monitors: a window parked on a hidden one and brought
/// back keeps the rect it had.
fn rehomed_rect(
    rect: Rectangle<i32, Logical>,
    decoration: i32,
    from: End,
    to: End,
) -> Rectangle<i32, Logical> {
    let both_shown = from.presence == Presence::Shown && to.presence == Presence::Shown;
    if rect.is_empty() || !both_shown {
        return rect;
    }
    placement::rehome(rect, from.area, to.area, decoration)
}

impl Shell {
    pub(super) fn origin_of(&self, at: Location) -> Option<Origin> {
        let monitor = self.monitor_by_id(at.output)?;
        Some(Origin {
            output: Some(monitor.output().clone()),
            area: self.workspace(at)?.area(),
            presence: monitor.config().presence,
        })
    }

    /// Settles a window that has just arrived on its current monitor from
    /// `from`: its surfaces enter the new output and its floating rect is
    /// carried into the new usable area.
    pub(super) fn rehome_window(&mut self, id: WindowId, from: &Origin) {
        let Some(location) = self.location(id) else {
            return;
        };
        let Some(monitor) = self.monitor_by_id(location.output) else {
            return;
        };
        let Some(area) = self.workspace(location).map(Workspace::area) else {
            return;
        };
        if let Some(tile) = self.tile(id) {
            self.transfer_window(tile.window(), from.output.as_ref(), monitor.output());
        }
        let to = End {
            presence: monitor.config().presence,
            area,
        };
        let from = End {
            presence: from.presence,
            area: from.area,
        };
        self.preferred_stale = true;
        let Some(tile) = self.tile_mut(id) else {
            return;
        };
        let rect = tile.floating_rect();
        let decoration = tile.insets().top;
        tile.set_floating_rect(rehomed_rect(rect, decoration, from, to));
        self.mark_dirty(id);
    }

    /// Whether a window changed monitor since the last call, so the
    /// surfaces' preferred colour description is out of date.
    #[expect(dead_code, reason = "the colour-management refresh is its reader")]
    pub fn take_preferred_stale(&mut self) -> bool {
        std::mem::take(&mut self.preferred_stale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::testing::area;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn shown(width: i32, height: i32) -> End {
        End {
            presence: Presence::Shown,
            area: area(width, height),
        }
    }

    fn hidden() -> End {
        End {
            presence: Presence::Hidden,
            area: area(3840, 2160),
        }
    }

    #[test]
    fn hiding_a_window_and_showing_it_again_gives_back_its_rect() {
        let before = rect(300, 200, 640, 480);
        let parked = rehomed_rect(before, 36, shown(1920, 1080), hidden());
        let back = rehomed_rect(parked, 36, hidden(), shown(1920, 1080));
        assert_eq!(back, before);
    }

    #[test]
    fn a_window_between_shown_monitors_is_carried() {
        let carried = rehomed_rect(
            rect(300, 200, 640, 480),
            36,
            shown(1920, 1080),
            shown(1280, 720),
        );
        assert_eq!(carried.size, (640, 480).into());
        assert!(area(1280, 720).contains_rect(carried));
    }

    #[test]
    fn a_window_that_never_floated_has_no_rect_to_carry() {
        let empty = Rectangle::default();
        assert_eq!(
            rehomed_rect(empty, 36, shown(1920, 1080), shown(1280, 720)),
            empty
        );
    }
}

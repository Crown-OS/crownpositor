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
    shell::{Location, Shell, workspace::Workspace},
    utils::id::WindowId,
};

/// Where a window comes from: the output it leaves, when it still has to
/// leave one, and the usable area its floating rect was measured in.
pub(super) struct Origin {
    pub output: Option<Output>,
    pub area: Rectangle<i32, Logical>,
}

impl Shell {
    pub(super) fn origin_of(&self, at: Location) -> Option<Origin> {
        Some(Origin {
            output: Some(self.monitor_by_id(at.output)?.output().clone()),
            area: self.workspace(at)?.area(),
        })
    }

    /// Settles a window that has just arrived on its current monitor from
    /// `from`: its surfaces enter the new output and its floating rect is
    /// carried into the new usable area. A hidden monitor keeps the rect as
    /// it was, so a window parked there comes back at its old size.
    pub(super) fn rehome_window(&mut self, id: WindowId, from: &Origin) {
        let Some(location) = self.location(id) else {
            return;
        };
        let Some(monitor) = self.monitor_by_id(location.output) else {
            return;
        };
        let Some(to_area) = self.workspace(location).map(Workspace::area) else {
            return;
        };
        if let Some(tile) = self.tile(id) {
            self.transfer_window(tile.window(), from.output.as_ref(), monitor.output());
        }
        let target_is_hidden = monitor.is_hidden();
        self.preferred_stale = true;
        if target_is_hidden {
            return;
        }
        let Some(tile) = self.tile_mut(id) else {
            return;
        };
        let rect = tile.floating_rect();
        if !rect.is_empty() {
            let decoration = tile.insets().top;
            tile.set_floating_rect(placement::rehome(rect, from.area, to_area, decoration));
        }
        self.mark_dirty(id);
    }

    /// Whether a window changed monitor since the last call, so the
    /// surfaces' preferred colour description is out of date.
    #[expect(dead_code, reason = "the colour-management refresh is its reader")]
    pub fn take_preferred_stale(&mut self) -> bool {
        std::mem::take(&mut self.preferred_stale)
    }
}

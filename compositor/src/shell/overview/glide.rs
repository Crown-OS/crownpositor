//! Showing a change of layout as motion.
//!
//! A relayout moves things in one step. What was on screen is remembered
//! first ([`Snapshot`]), and everything that moved glides from there to where
//! the layout now puts it — the layout stays the one source of truth about
//! where things are.

use smithay::utils::{Logical, Rectangle};

use spacecontrol::scene::Slot;

use super::SpaceControl;
use crate::utils::id::{WindowId, WorkspaceId};

impl SpaceControl {
    /// Where preview `index` stands while another is dragged: the slot it
    /// steps into to make room for it.
    pub(super) fn room_target(&self, index: usize) -> Option<Slot> {
        let (from, to) = self.making_room?;
        let step = match (from < to, from > to) {
            (true, _) if index > from && index <= to => index - 1,
            (_, true) if index >= to && index < from => index + 1,
            _ => return None,
        };
        self.bar.get(step).copied()
    }

    /// Sends a window let go over nothing back to its cell, at the speed it
    /// was let go with.
    pub fn return_window(
        &mut self,
        index: usize,
        from: Rectangle<f64, Logical>,
        velocity: (f64, f64),
    ) {
        self.flip = None;
        let Some(page) = self.pages.get(self.active) else {
            return;
        };
        let (Some(id), Some(cell)) = (page.windows.get(index), page.grid.get(index)) else {
            return;
        };
        self.window_glides.launch(*id, from, *cell, velocity);
    }

    /// Where every window and preview is drawn right now — a carried window
    /// where the hand has it, not the cell it left.
    pub(super) fn snapshot(&self) -> Snapshot {
        let carried = self.carrying();
        let windows = self
            .pages
            .iter()
            .enumerate()
            .flat_map(|(workspace, page)| {
                page.windows
                    .iter()
                    .enumerate()
                    .filter_map(move |(slot, id)| {
                        let rect = match carried {
                            Some((window, rect)) if workspace == self.active && window == slot => {
                                rect
                            }
                            _ => self.grid_rect(workspace, slot)?,
                        };
                        Some((*id, rect))
                    })
            })
            .collect();
        let previews = (0..self.bar.len())
            .filter_map(|index| Some((self.workspace_id(index)?, self.preview_slot(index)?.thumb)))
            .collect();
        Snapshot { windows, previews }
    }

    /// Glides everything that moved from where `before` had it to where the
    /// layout puts it now.
    pub(super) fn glide_from(&mut self, before: Snapshot) {
        for (id, from) in before.windows {
            if let Some(to) = self.pages.iter().find_map(|page| {
                page.windows
                    .iter()
                    .position(|it| *it == id)
                    .map(|slot| page.grid[slot])
            }) {
                self.window_glides.launch(id, from, to, (0.0, 0.0));
            }
        }
        for (id, from) in before.previews {
            if let Some(to) = self
                .pages
                .iter()
                .position(|page| page.workspace == Some(id))
                .and_then(|index| self.bar.get(index))
            {
                self.preview_glides.launch(id, from, to.thumb, (0.0, 0.0));
            }
        }
    }

    /// Steps the previews aside for one being dragged along the bar, gliding
    /// each from wherever it is to the place it now stands in.
    pub(super) fn make_room(&mut self) {
        let room = self
            .input
            .reordering()
            .map(|reorder| (reorder.workspace, reorder.slot));
        if room == self.making_room {
            return;
        }
        let before: Vec<_> = (0..self.bar.len())
            .filter_map(|index| Some((index, self.preview_slot(index)?.thumb)))
            .collect();
        self.making_room = room;
        for (index, from) in before {
            let Some(id) = self.workspace_id(index) else {
                continue;
            };
            let Some(to) = self
                .room_target(index)
                .or_else(|| self.bar.get(index).copied())
            else {
                continue;
            };
            self.preview_glides.launch(id, from, to.thumb, (0.0, 0.0));
        }
    }
}

/// Where every window and preview was drawn at one moment, by identity.
#[derive(Debug, Default)]
pub(super) struct Snapshot {
    windows: Vec<(WindowId, Rectangle<f64, Logical>)>,
    previews: Vec<(WorkspaceId, Rectangle<f64, Logical>)>,
}

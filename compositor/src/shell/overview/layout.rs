//! The layout an overview shows, cached against what it was solved from.
//!
//! Every workspace is laid out, not just the active one: a swipe across the
//! overview slides a neighbour's grid in, and it has to be somewhere already
//! for the slide to cost nothing but a different destination rectangle.
//!
//! Nothing about the layout changes while the overview animates — the windows
//! fly towards rectangles fixed the moment it opened — so it is solved once and
//! [`Fingerprint`] notices when that stops being true.

use smithay::utils::{Logical, Rectangle};

use spacecontrol::scene::{self, Canvas};

use super::SpaceControl;
use crate::{
    shell::monitor::Monitor,
    utils::id::{WindowId, WorkspaceId},
};

/// What the cached layout was computed from. When this changes the layout is
/// stale — a window opened, closed, resized, or the output did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Fingerprint {
    output: (i32, i32, i32, i32),
    usable: (i32, i32, i32, i32),
    workspaces: usize,
    active: usize,
    /// Every window's identity and size on every workspace, folded together.
    /// Order matters: a window raised above another re-reads the grid.
    windows: u64,
}

impl Fingerprint {
    fn of(monitor: &Monitor) -> Self {
        let geometry = monitor.geometry();
        let usable = monitor.usable();

        let windows = monitor
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.tiles())
            .fold(0xcbf2_9ce4_8422_2325, |hash, tile| {
                let size = tile.target().size;
                let mixed = tile.id().raw()
                    ^ (u64::from(size.w as u32) << 20)
                    ^ (u64::from(size.h as u32) << 40);
                (hash ^ mixed).wrapping_mul(0x1000_0000_01b3)
            });

        let corners =
            |rect: Rectangle<i32, Logical>| (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);

        Self {
            output: corners(geometry),
            usable: corners(usable),
            workspaces: monitor.workspaces().len(),
            active: monitor.active_index(),
            windows,
        }
    }
}

/// One workspace's thumbnails, all index-aligned.
#[derive(Debug, Default)]
pub(super) struct Page {
    pub(super) workspace: Option<WorkspaceId>,
    /// Where each window lands once the overview is fully open.
    pub(super) grid: Vec<Rectangle<f64, Logical>>,
    pub(super) windows: Vec<WindowId>,
    /// Where each window sits on the desktop, which is what a preview shrinks.
    pub(super) desktop: Vec<Rectangle<i32, Logical>>,
}

impl SpaceControl {
    /// Recomputes the grid and the bar if anything they depend on moved.
    ///
    /// Called once per frame while the overview is on screen; the fingerprint
    /// makes all but the first of those free.
    pub fn relayout(&mut self, monitor: &Monitor) {
        let fingerprint = Fingerprint::of(monitor);
        if fingerprint == self.fingerprint && !self.pages.is_empty() {
            return;
        }
        self.fingerprint = fingerprint;
        self.resolve(monitor);
    }

    /// Recomputes unconditionally — for when the model changed under a
    /// fingerprint that happens to match, such as a window moving workspace.
    pub fn resolve(&mut self, monitor: &Monitor) {
        // Only an overview already on screen glides: one opening flies its
        // windows in from the desktop instead.
        let before = match self.overview.is_open() {
            true => Some(self.flip.take().unwrap_or_else(|| self.snapshot())),
            false => None,
        };
        self.canvas = Canvas::new(monitor.geometry(), monitor.usable());
        self.active = monitor.active_index();

        self.pages
            .resize_with(monitor.workspaces().len(), Page::default);

        for (workspace, page) in monitor.workspaces().iter().zip(&mut self.pages) {
            page.workspace = Some(workspace.id());
            page.windows.clear();
            page.desktop.clear();
            self.sizes.clear();
            for tile in workspace.tiles() {
                page.windows.push(tile.id());
                page.desktop.push(tile.target());
                self.sizes.push(tile.target().size);
            }
            scene::grid(self.canvas, &self.sizes, &self.metrics, &mut page.grid);
        }

        // One slot more than there are workspaces: the '+' tile, laid out with
        // the previews so the bar stays centred with it in.
        scene::bar(
            self.canvas,
            monitor.workspaces().len() + 1,
            &self.metrics,
            &mut self.bar,
        );
        self.add = self.bar.pop();
        self.ids.ensure(self.bar.len());
        self.windows_hover.resize(self.grid().len());
        self.workspaces_hover.resize(self.bar.len() + 1);
        self.making_room = None;

        if let Some(before) = before {
            self.glide_from(before);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_notices_a_resized_window() {
        let a = Fingerprint {
            output: (0, 0, 1920, 1080),
            usable: (0, 32, 1920, 1048),
            workspaces: 2,
            active: 0,
            windows: 11,
        };
        assert_eq!(a, a);
        assert_ne!(a, Fingerprint { windows: 12, ..a });
        assert_ne!(a, Fingerprint { workspaces: 3, ..a });
        assert_ne!(a, Fingerprint { active: 1, ..a });
        assert_ne!(
            a,
            Fingerprint {
                usable: (0, 0, 1920, 1080),
                ..a
            }
        );
        assert_ne!(
            a,
            Fingerprint {
                output: (0, 0, 2560, 1080),
                ..a
            }
        );
    }
}

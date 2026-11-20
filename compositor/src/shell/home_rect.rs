//! Where a migrated floating window sat on the monitor it left, so it goes
//! back to exactly that place when the monitor returns. Carrying the rect
//! there and back through `placement::rehome` rounds it and clamps its size
//! each way, and measures it against a usable area the returning monitor's
//! panels may not have reserved yet.

use smithay::utils::{Logical, Rectangle};

use crate::{
    layout::placement,
    shell::{
        Shell,
        migration::{Departing, Migration},
        workspace::Workspace,
    },
    utils::id::{OutputId, WindowId},
};

/// A floating window's rect in the space of the monitor it left.
pub type HomeRect = (WindowId, Rectangle<i32, Logical>);

/// The home rects of the windows `record` takes back, read through
/// `floating_rect`. Tiled windows have no rect and need none.
pub fn home_rects(
    record: &Migration,
    departing: &[Departing],
    floating_rect: impl Fn(WindowId) -> Option<Rectangle<i32, Logical>>,
) -> Vec<HomeRect> {
    let windows: Vec<WindowId> = match record {
        Migration::Merged(windows) => windows.iter().map(|(_, id)| *id).collect(),
        Migration::Workspaces { workspaces, .. } => departing
            .iter()
            .filter(|workspace| workspaces.contains(&workspace.id))
            .flat_map(|workspace| workspace.windows.iter().copied())
            .collect(),
    };
    windows
        .into_iter()
        .filter_map(|id| Some((id, floating_rect(id).filter(|rect| !rect.is_empty())?)))
        .collect()
}

/// The home rects of the windows that are back on `target`. A window the
/// user moved away while its monitor was gone stays where it was put.
pub fn reclaimed<'a>(
    homes: &'a [HomeRect],
    target: OutputId,
    output_of: impl Fn(WindowId) -> Option<OutputId> + 'a,
) -> impl Iterator<Item = HomeRect> + 'a {
    homes
        .iter()
        .copied()
        .filter(move |(id, _)| output_of(*id) == Some(target))
}

impl Shell {
    /// Puts each window reclaimed onto `target` back where it sat before it
    /// left, when that place can still be reached on the output. A window
    /// that sat partly off the screen goes back there too.
    pub(super) fn restore_home_rects(&mut self, homes: &[HomeRect], target: OutputId) {
        let back: Vec<HomeRect> = reclaimed(homes, target, |id| {
            self.location(id).map(|location| location.output)
        })
        .collect();
        for (id, rect) in back {
            let Some(output_area) = self
                .location(id)
                .and_then(|location| self.workspace(location))
                .map(Workspace::output_area)
            else {
                continue;
            };
            let Some(tile) = self.tile_mut(id) else {
                continue;
            };
            if placement::keep_reachable(rect, output_area, tile.insets().top) == rect {
                tile.set_floating_rect(rect);
                self.mark_dirty(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::id::WorkspaceId;

    #[test]
    fn only_the_windows_back_on_the_returning_monitor_go_home() {
        let (back, moved_away) = (WindowId::next(), WindowId::next());
        let (returning, survivor) = (OutputId::from_raw(2), OutputId::from_raw(3));
        let homes = [(back, rect(36)), (moved_away, rect(80))];
        let restored: Vec<HomeRect> = reclaimed(&homes, returning, |id| {
            Some(if id == back { returning } else { survivor })
        })
        .collect();
        assert_eq!(restored, vec![(back, rect(36))]);
    }

    fn rect(y: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((8, y).into(), (217, 365).into())
    }

    #[test]
    fn a_merged_record_keeps_the_rect_of_each_floating_window_it_takes_back() {
        let (floating, tiled, foreign) = (WindowId::next(), WindowId::next(), WindowId::next());
        let record = Migration::Merged(vec![(0, floating), (1, tiled)]);
        let homes = home_rects(&record, &[], |id| match id {
            id if id == floating => Some(rect(36)),
            id if id == foreign => Some(rect(50)),
            _ => Some(Rectangle::default()),
        });
        assert_eq!(homes, vec![(floating, rect(36))]);
    }

    #[test]
    fn a_workspace_record_keeps_the_rects_on_the_workspaces_it_takes_back() {
        let (kept, left_behind) = (WindowId::next(), WindowId::next());
        let departing = [
            Departing {
                id: WorkspaceId::next(),
                windows: vec![kept],
                active: true,
            },
            Departing {
                id: WorkspaceId::next(),
                windows: vec![left_behind],
                active: false,
            },
        ];
        let record = Migration::Workspaces {
            workspaces: vec![departing[0].id],
            active: Some(departing[0].id),
        };
        assert_eq!(
            home_rects(&record, &departing, |_| Some(rect(36))),
            vec![(kept, rect(36))]
        );
    }
}

//! Where a migrated floating window sat on the monitor it left, so it goes
//! back to exactly that place when the monitor returns. Carrying the rect
//! there and back through `placement::rehome` rounds it and clamps its size
//! each way, and measures it against a usable area the returning monitor's
//! panels may not have reserved yet.

use smithay::utils::{Logical, Rectangle};

use crate::{
    shell::{
        Shell,
        migration::{Departing, Migration},
    },
    utils::id::WindowId,
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

impl Shell {
    /// Puts each reclaimed window back where it sat before it left, when
    /// that place still lies on its output.
    pub(super) fn restore_home_rects(&mut self, homes: &[HomeRect]) {
        for &(id, rect) in homes {
            let fits = self
                .location(id)
                .and_then(|location| self.workspace(location))
                .is_some_and(|workspace| workspace.output_area().contains_rect(rect));
            if let Some(tile) = self.tile_mut(id).filter(|_| fits) {
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

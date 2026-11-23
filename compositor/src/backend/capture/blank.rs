//! Sensitive windows drawn as black boxes in screen, workspace and window
//! captures, so a recording of the whole output carries no password field
//! or private window either. A box covers the window where it is drawn this
//! frame, its popups, its closing animation, and every workspace a slide
//! shows.

use std::collections::{HashMap, HashSet};

use smithay::{
    backend::renderer::element::{Id, Kind, solid::SolidColorRenderElement},
    desktop::PopupManager,
    reexports::wayland_server::{Resource, backend::ObjectId, protocol::wl_surface::WlSurface},
    utils::{Logical, Physical, Point, Rectangle, Scale},
};

use crate::{
    backend::capture::source::{Placement, View},
    rendering::Placed,
    shell::{tile::Tile, workspace::Workspace},
    utils::id::WindowId,
};

/// An element id per sensitive window and per popup of one, kept while the
/// window stays sensitive or closes, so the damage of a capture settles.
#[derive(Debug, Default)]
pub struct BlankIds {
    windows: HashMap<WindowId, Id>,
    popups: HashMap<ObjectId, Id>,
}

impl BlankIds {
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Keeps the ids of `blanked` windows and `popups`, and gives new
    /// `sensitive` windows and popups one.
    pub fn keep(
        &mut self,
        blanked: &HashSet<WindowId>,
        sensitive: &HashSet<WindowId>,
        popups: HashSet<ObjectId>,
    ) {
        self.windows.retain(|id, _| blanked.contains(id));
        for id in sensitive {
            self.windows.entry(*id).or_insert_with(Id::new);
        }
        self.popups.retain(|surface, _| popups.contains(surface));
        for surface in popups {
            self.popups.entry(surface).or_insert_with(Id::new);
        }
    }

    fn popup(&self, surface: &WlSurface) -> Id {
        self.popups
            .get(&surface.id())
            .cloned()
            .unwrap_or_else(Id::new)
    }
}

const BLANK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Opaque boxes over each sensitive window the captured image shows.
pub fn sensitive_blanks(
    placement: &Placement<'_>,
    blanks: &BlankIds,
    scale: Scale<f64>,
) -> Vec<SolidColorRenderElement> {
    if blanks.is_empty() {
        return Vec::new();
    }
    let canvas = Rectangle::from_size(placement.area.size.to_physical_precise_round(scale));
    let boxes: Vec<SolidColorRenderElement> = match placement.view {
        View::Window(tile) => blanks
            .windows
            .get(&tile.id())
            .map(|id| blank(id.clone(), canvas))
            .into_iter()
            .collect(),
        View::Workspace(workspace) => {
            workspace_blanks(workspace, Point::default(), blanks, scale, canvas)
        }
        View::Output => placement
            .monitor
            .visible_workspaces()
            .flat_map(|(workspace, offset)| {
                workspace_blanks(workspace, offset, blanks, scale, canvas)
            })
            .collect(),
    };
    boxes
}

/// A window that morphs, into its icon or out of it, is drawn through warps
/// that reach outside its frame, so the whole image is covered meanwhile.
fn workspace_blanks(
    workspace: &Workspace,
    offset: Point<f64, Logical>,
    blanks: &BlankIds,
    scale: Scale<f64>,
    canvas: Rectangle<i32, Physical>,
) -> Vec<SolidColorRenderElement> {
    let shown = workspace
        .tiles()
        .iter()
        .filter(|tile| tile.is_shown() || tile.morph().is_some());
    let closing = workspace.closing().iter().map(|closing| closing.tile());
    shown
        .chain(closing)
        .filter_map(|tile| blanks.windows.get(&tile.id()).map(|id| (tile, id)))
        .flat_map(|(tile, id)| {
            if tile.morph().is_some() {
                vec![blank(id.clone(), canvas)]
            } else {
                tile_blanks(tile, id, offset, blanks, scale)
            }
        })
        .collect()
}

/// The window's frame where it is drawn this frame, and each of its popups.
fn tile_blanks(
    tile: &Tile,
    id: &Id,
    offset: Point<f64, Logical>,
    blanks: &BlankIds,
    scale: Scale<f64>,
) -> Vec<SolidColorRenderElement> {
    let placed = Placed::new(tile, offset, scale);
    let mut boxes = vec![blank(id.clone(), placed.frame)];
    let Some(toplevel) = tile.window().toplevel() else {
        return boxes;
    };
    let geometry_origin = tile.window().geometry().loc;
    for (popup, popup_offset) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
        let location = placed.surface_origin
            + (geometry_origin + popup_offset).to_physical_precise_round(scale);
        let size = popup.geometry().size.to_physical_precise_round(scale);
        boxes.push(blank(
            blanks.popup(popup.wl_surface()),
            Rectangle::new(location, size),
        ));
    }
    boxes
}

fn blank(id: Id, geometry: Rectangle<i32, Physical>) -> SolidColorRenderElement {
    SolidColorRenderElement::new(id, geometry, 0usize, BLANK, Kind::Unspecified)
}

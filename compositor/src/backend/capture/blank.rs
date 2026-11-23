//! Sensitive windows drawn as black boxes in screen, workspace and window
//! captures, so a recording of the whole output carries no password field
//! or private window either. A box covers the window where it is drawn this
//! frame, its popups, its closing animation, and every workspace a slide
//! shows.

use std::collections::HashMap;

use smithay::{
    backend::renderer::element::{Id, Kind, solid::SolidColorRenderElement},
    desktop::PopupManager,
    utils::{Logical, Physical, Point, Rectangle, Scale},
};

use crate::{
    backend::capture::source::{Placement, View},
    rendering::Placed,
    shell::{tile::Tile, workspace::Workspace},
    utils::id::WindowId,
};

/// An element id per sensitive window, kept while it stays sensitive or
/// closes, so the damage of a capture does not churn.
pub type BlankIds = HashMap<WindowId, Id>;

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
    match placement.view {
        View::Window(tile) => blanks
            .get(&tile.id())
            .map(|id| {
                let canvas =
                    Rectangle::from_size(placement.area.size.to_physical_precise_round(scale));
                blank(id.clone(), canvas)
            })
            .into_iter()
            .collect(),
        View::Workspace(workspace) => workspace_blanks(workspace, Point::default(), blanks, scale),
        View::Output => placement
            .monitor
            .visible_workspaces()
            .flat_map(|(workspace, offset)| workspace_blanks(workspace, offset, blanks, scale))
            .collect(),
    }
}

fn workspace_blanks(
    workspace: &Workspace,
    offset: Point<f64, Logical>,
    blanks: &BlankIds,
    scale: Scale<f64>,
) -> Vec<SolidColorRenderElement> {
    let shown = workspace.tiles().iter().filter(|tile| !tile.is_minimized());
    let closing = workspace.closing().iter().map(|closing| closing.tile());
    shown
        .chain(closing)
        .filter_map(|tile| blanks.get(&tile.id()).map(|id| (tile, id)))
        .flat_map(|(tile, id)| tile_blanks(tile, id, offset, scale))
        .collect()
}

/// The window's frame where it is drawn this frame, and each of its popups.
fn tile_blanks(
    tile: &Tile,
    id: &Id,
    offset: Point<f64, Logical>,
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
        boxes.push(blank(Id::new(), Rectangle::new(location, size)));
    }
    boxes
}

fn blank(id: Id, geometry: Rectangle<i32, Physical>) -> SolidColorRenderElement {
    SolidColorRenderElement::new(id, geometry, 0usize, BLANK, Kind::Unspecified)
}

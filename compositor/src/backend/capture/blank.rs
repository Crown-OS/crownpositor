//! Sensitive windows drawn as black boxes in screen, workspace and window
//! captures, so a recording of the whole output carries no password field
//! or private window either.

use std::collections::HashMap;

use smithay::{
    backend::renderer::element::{Id, Kind, solid::SolidColorRenderElement},
    utils::{Logical, Physical, Rectangle, Scale},
};

use crate::{
    backend::capture::source::{Placement, View},
    shell::Shell,
    utils::id::WindowId,
};

/// An element id per sensitive window, kept while it stays sensitive so the
/// damage of a capture does not churn.
pub type BlankIds = HashMap<WindowId, Id>;

const BLANK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// One opaque box over each sensitive window the captured image shows.
pub fn sensitive_blanks(
    shell: &Shell,
    placement: &Placement<'_>,
    blanks: &BlankIds,
    scale: Scale<f64>,
) -> Vec<SolidColorRenderElement> {
    if blanks.is_empty() {
        return Vec::new();
    }
    let area = placement.area;
    let element = |id: &Id, rect: Rectangle<i32, Logical>| {
        let local = Rectangle::<i32, Logical>::new(rect.loc - area.loc, rect.size);
        let geometry: Rectangle<i32, Physical> = local.to_physical_precise_round(scale);
        SolidColorRenderElement::new(id.clone(), geometry, 0usize, BLANK, Kind::Unspecified)
    };
    if let View::Window(tile) = placement.view {
        return blanks
            .get(&tile.id())
            .map(|id| element(id, area))
            .into_iter()
            .collect();
    }
    let workspace = match placement.view {
        View::Workspace(workspace) => workspace.id(),
        View::Output | View::Window(_) => placement.monitor.active().id(),
    };
    shell
        .tiles()
        .filter(|tile| !tile.is_minimized())
        .filter(|tile| {
            shell.location(tile.id()).is_some_and(|location| {
                location.output == placement.monitor.id() && location.workspace == workspace
            })
        })
        .filter_map(|tile| blanks.get(&tile.id()).map(|id| element(id, tile.target())))
        .collect()
}

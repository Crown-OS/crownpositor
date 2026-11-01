//! Where windows come from and go back to: the icons a launcher registered
//! through `crownos_app_icons_v1`, turned into each window's motion.

use config::{AnimationProfile, CloseAnimation, MinimizeAnimation};
use protocols::crownos_app_icons::{AppWindows, PlacedIcon};
use smithay::{
    desktop::{WindowSurfaceType, layer_map_for_output},
    output::Output,
    utils::{Rectangle, Scale},
    wayland::shell::wlr_layer::Anchor,
};

use crate::{
    rendering::snapshot::Snapshot,
    shell::{
        closing::Farewell,
        dust::Dust,
        morph::{Edge, IconOrigin},
    },
    state::State,
    utils::{
        id::{OutputId, WindowId},
        surface,
    },
};

impl State {
    /// The icon of `id`'s application on the output the window is on, in the
    /// window's own coordinates.
    pub fn icon_of(&self, id: WindowId) -> Option<IconOrigin> {
        let app_id = surface::app_id(self.shell.tile(id)?.surface())?;
        let location = self.shell.location(id)?;
        let output = self.shell.monitor_by_id(location.output)?.output();
        self.wayland
            .app_icons_state
            .icons_of(&app_id)
            .find_map(|icon| icon_on(output, icon))
    }

    /// Where `icon` is on screen now, and which output it is on.
    pub fn icon_at(&self, icon: &PlacedIcon) -> Option<(OutputId, IconOrigin)> {
        self.shell.monitors().iter().find_map(|monitor| {
            icon_on(monitor.output(), icon.clone()).map(|origin| (monitor.id(), origin))
        })
    }

    /// Tells launchers how many windows each application has, and lifts the
    /// icons windows' motions carry out of them.
    pub fn refresh_app_icons(&mut self) {
        let focused = self.shell.focused_window_id();
        let mut apps = Vec::new();
        for tile in self.shell.tiles() {
            if surface::has_parent(tile.surface()) {
                continue;
            }
            surface::with_app_id(tile.surface(), |app_id| {
                if !app_id.is_empty() {
                    AppWindows::count_into(&mut apps, app_id, Some(tile.id()) == focused);
                }
            });
        }
        let carried = match self.config.current.appearance.minimize_effect {
            MinimizeAnimation::Liquid => self.shell.carried_icons(),
            MinimizeAnimation::Genie | MinimizeAnimation::Scale => Vec::new(),
        };
        let icons = &mut self.wayland.app_icons_state;
        icons.publish_windows(apps);
        icons.carry(&carried);
    }

    /// Where a window minimizes into: its icon, or where a dock would be.
    fn minimize_origin(&self, id: WindowId) -> Option<IconOrigin> {
        self.icon_of(id).or_else(|| {
            let location = self.shell.location(id)?;
            let workspace = self.shell.workspace(location)?;
            Some(IconOrigin::bottom_of(workspace.output_area()))
        })
    }

    /// How a closed window leaves: an application window crumbles to dust or
    /// shrinks into its icon, as configured; a dialog shrinks away where it
    /// stands.
    fn farewell(&self, id: WindowId) -> Option<Farewell> {
        let tile = self.shell.tile(id)?;
        let appearance = &self.config.current.appearance;
        let dialog = surface::has_parent(tile.surface());
        if !dialog {
            if appearance.close_effect == CloseAnimation::Dust
                && appearance.animations != AnimationProfile::None
            {
                return Some(Farewell::Dust(Dust::new(id.raw())));
            }
            if let Some(icon) = self.icon_of(id) {
                return Some(Farewell::Shrink(icon));
            }
        }
        let radius = f64::from(appearance.border_radius);
        Some(Farewell::Shrink(IconOrigin::in_place(
            tile.render_rect(),
            radius,
        )))
    }

    pub fn minimize_window(&mut self, id: WindowId) {
        let Some(origin) = self.minimize_origin(id) else {
            return;
        };
        if self.shell.minimize_window(id, origin) {
            self.shell.refresh();
            self.update_keyboard_focus();
            self.queue_redraw();
        }
    }

    /// Grows a window that just mapped out of the icon its application was
    /// launched from, if it was.
    pub fn grow_if_launched(&mut self, id: WindowId) {
        let Some(tile) = self.shell.tile(id) else {
            return;
        };
        if surface::has_parent(tile.surface()) {
            return;
        }
        let Some(app_id) = surface::app_id(tile.surface()) else {
            return;
        };
        let Some(from) = self.wayland.launches.take(&app_id) else {
            return;
        };
        let output = self.shell.location(id).map(|location| location.output);
        let origin = from
            .filter(|(launched_on, _)| Some(*launched_on) == output)
            .map(|(_, icon)| icon)
            .or_else(|| self.icon_of(id));
        if let (Some(origin), Some(tile)) = (origin, self.shell.tile_mut(id)) {
            tile.grow_from(origin);
        }
    }

    /// Takes a window whose client is closing it off screen through its icon,
    /// from a snapshot of its last frame. `false` when it could not be kept,
    /// and the caller removes it outright.
    pub fn close_window(&mut self, id: WindowId) -> bool {
        let Some(tile) = self.shell.tile(id).filter(|tile| tile.is_shown()) else {
            return false;
        };
        let Some(scale) = self
            .shell
            .location(id)
            .and_then(|location| self.shell.monitor_by_id(location.output))
            .map(|monitor| Scale::from(monitor.output().current_scale().fractional_scale()))
        else {
            return false;
        };
        let window = tile.window().clone();
        let Some(farewell) = self.farewell(id) else {
            return false;
        };
        let Some(snapshot) = self
            .backend
            .with_gles(|renderer| Snapshot::capture(renderer, &window, scale))
            .flatten()
        else {
            return false;
        };
        self.shell.close_into(id, snapshot, farewell)
    }
}

/// `icon`, if the surface it is drawn on is a layer on `output`.
fn icon_on(output: &Output, icon: PlacedIcon) -> Option<IconOrigin> {
    let layers = layer_map_for_output(output);
    let layer = layers.layer_for_surface(&icon.surface, WindowSurfaceType::TOPLEVEL)?;
    let placement = icon.placement;
    let loc = layers.layer_geometry(layer)?.loc + placement.rect.loc;
    let edge = dock_edge(layer.cached_state().anchor);
    drop(layers);
    Some(IconOrigin {
        rect: Rectangle::new(loc.to_f64(), placement.rect.size.to_f64()),
        radius: placement.corner_radius,
        edge,
        carrier: Some(icon),
    })
}

/// The one edge a layer is anchored along: a dock pinned to the bottom,
/// stretched or not. `None` for a surface anchored to a corner or to all four
/// sides, which lies along no edge in particular.
fn dock_edge(anchor: Anchor) -> Option<Edge> {
    let vertical = match (
        anchor.contains(Anchor::TOP),
        anchor.contains(Anchor::BOTTOM),
    ) {
        (true, false) => Some(Edge::Top),
        (false, true) => Some(Edge::Bottom),
        _ => None,
    };
    let horizontal = match (
        anchor.contains(Anchor::LEFT),
        anchor.contains(Anchor::RIGHT),
    ) {
        (true, false) => Some(Edge::Left),
        (false, true) => Some(Edge::Right),
        _ => None,
    };
    match (vertical, horizontal) {
        (Some(edge), None) | (None, Some(edge)) => Some(edge),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dock_lies_along_the_edge_it_is_pinned_to() {
        assert_eq!(dock_edge(Anchor::BOTTOM), Some(Edge::Bottom));
        assert_eq!(
            dock_edge(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT),
            Some(Edge::Bottom)
        );
        assert_eq!(
            dock_edge(Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM),
            Some(Edge::Left)
        );
    }

    #[test]
    fn a_launcher_over_the_screen_lies_along_no_edge() {
        assert_eq!(dock_edge(Anchor::all()), None);
        assert_eq!(dock_edge(Anchor::BOTTOM | Anchor::LEFT), None);
        assert_eq!(dock_edge(Anchor::empty()), None);
    }
}

//! Where windows come from and go back to: the icons a launcher registered
//! through `crownos_app_icons_v1`, turned into each window's motion.

use smithay::{
    desktop::{WindowSurfaceType, layer_map_for_output},
    utils::{Rectangle, Scale},
};

use crate::{
    rendering::snapshot::Snapshot,
    shell::morph::IconOrigin,
    state::State,
    utils::{id::WindowId, surface},
};

impl State {
    /// The icon of `id`'s application on the output the window is on, in the
    /// window's own coordinates.
    pub fn icon_of(&self, id: WindowId) -> Option<IconOrigin> {
        let app_id = surface::app_id(self.shell.tile(id)?.surface())?;
        let location = self.shell.location(id)?;
        let output = self.shell.monitor_by_id(location.output)?.output();
        let layers = layer_map_for_output(output);
        self.wayland
            .app_icons_state
            .icons_of(&app_id)
            .find_map(|(surface, placement)| {
                let layer = layers.layer_for_surface(&surface, WindowSurfaceType::TOPLEVEL)?;
                let loc = layers.layer_geometry(layer)?.loc + placement.rect.loc;
                Some(IconOrigin {
                    rect: Rectangle::new(loc.to_f64(), placement.rect.size.to_f64()),
                    radius: placement.corner_radius,
                })
            })
    }

    /// Where a window minimizes into: its icon, or where a dock would be.
    fn minimize_origin(&self, id: WindowId) -> Option<IconOrigin> {
        self.icon_of(id).or_else(|| {
            let location = self.shell.location(id)?;
            let workspace = self.shell.workspace(location)?;
            Some(IconOrigin::bottom_of(workspace.output_area()))
        })
    }

    /// Where a closed window shrinks into: its icon, unless it is a dialog,
    /// which shrinks away where it stands.
    fn close_origin(&self, id: WindowId) -> Option<IconOrigin> {
        let tile = self.shell.tile(id)?;
        if !surface::has_parent(tile.surface())
            && let Some(icon) = self.icon_of(id)
        {
            return Some(icon);
        }
        let radius = f64::from(self.config.current.appearance.border_radius);
        Some(IconOrigin::in_place(tile.render_rect(), radius))
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
        if !self.wayland.app_icons_state.take_launch(&app_id) {
            return;
        }
        let origin = self.icon_of(id);
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
        let Some(origin) = self.close_origin(id) else {
            return false;
        };
        let Some(snapshot) = self
            .backend
            .with_gles(|renderer| Snapshot::capture(renderer, &window, scale))
            .flatten()
        else {
            return false;
        };
        self.shell.close_into(id, snapshot, origin)
    }
}

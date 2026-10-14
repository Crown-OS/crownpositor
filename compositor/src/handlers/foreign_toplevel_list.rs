use std::collections::HashMap;

use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
};

use crate::{
    state::State,
    utils::{id::WindowId, surface::with_app_id},
};

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.wayland.foreign_toplevel_list_state
    }
}

/// The `ext_foreign_toplevel_handle_v1` of every mapped window, and what its
/// clients were last told.
#[derive(Default)]
pub struct ForeignToplevels {
    handles: HashMap<WindowId, Announced>,
}

struct Announced {
    handle: ForeignToplevelHandle,
    title: String,
    app_id: String,
}

impl ForeignToplevels {
    /// The window a client named by its `identifier`, e.g. to capture it.
    pub fn window(&self, identifier: &str) -> Option<WindowId> {
        self.handles
            .iter()
            .find(|(_, announced)| announced.handle.identifier() == identifier)
            .map(|(id, _)| *id)
    }

    pub fn handle(&self, window: WindowId) -> Option<&ForeignToplevelHandle> {
        self.handles.get(&window).map(|announced| &announced.handle)
    }
}

impl State {
    /// Announces new windows, renames retitled ones and closes gone ones.
    /// Runs every loop iteration, so it allocates only when something changed.
    pub fn refresh_foreign_toplevels(&mut self) {
        let State { shell, wayland, .. } = self;
        let toplevels = &mut wayland.foreign_toplevels;
        let list = &mut wayland.foreign_toplevel_list_state;

        for tile in shell.tiles() {
            let title = tile.title();
            with_app_id(tile.surface(), |app_id| {
                match toplevels.handles.get_mut(&tile.id()) {
                    Some(announced) => {
                        let mut changed = false;
                        if announced.title != title {
                            announced.handle.send_title(title);
                            announced.title = title.to_owned();
                            changed = true;
                        }
                        if announced.app_id != app_id {
                            announced.handle.send_app_id(app_id);
                            announced.app_id = app_id.to_owned();
                            changed = true;
                        }
                        if changed {
                            announced.handle.send_done();
                        }
                    }
                    None => {
                        let handle = list.new_toplevel::<State>(title, app_id);
                        handle.send_done();
                        toplevels.handles.insert(
                            tile.id(),
                            Announced {
                                handle,
                                title: title.to_owned(),
                                app_id: app_id.to_owned(),
                            },
                        );
                    }
                }
            });
        }

        if toplevels.handles.len() != shell.tiles().count() {
            toplevels.handles.retain(|id, announced| {
                let alive = shell.tile(*id).is_some();
                if !alive {
                    list.remove_toplevel(&announced.handle);
                }
                alive
            });
        }
    }
}

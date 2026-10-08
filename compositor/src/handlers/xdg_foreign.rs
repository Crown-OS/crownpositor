use smithay::wayland::xdg_foreign::{XdgForeignHandler, XdgForeignState};

use crate::state::State;

/// `zxdg_exporter_v2`: lets a client hand out a handle to its window so
/// another process can parent a dialog to it — the portal's file chooser over
/// Chromium, most often.
impl XdgForeignHandler for State {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.wayland.xdg_foreign_state
    }
}

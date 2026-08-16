use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface, WlrLayerShellHandler, WlrLayerShellState,
};

use crate::state::State;

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.shell.layer_shell
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
    }
}

smithay::delegate_layer_shell!(State);

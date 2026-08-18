use smithay::desktop::{layer_map_for_output, LayerSurface};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState,
};

use tracing::warn;

use crate::state::State;

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.shell.layer_shell
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        match wl_output {
            Some(output) => {
                if let Some(output) = Output::from_resource(&output)
                    .filter(|output| self.output.outputs.contains_key(output))
                {
                    let mut map = layer_map_for_output(&output);
                    map.map_layer(&LayerSurface::new(surface, namespace))
                        .unwrap();
                }
            }
            None => {
                warn!("no output for new layer surface, closing");
                surface.send_close();
            }
        }
    }
}

smithay::delegate_layer_shell!(State);

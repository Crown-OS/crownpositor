use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer::WlBuffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, is_sync_subsurface,
        },
    },
};

use crate::{
    handlers::{drm_syncobj, layer_shell, xdg_shell},
    state::{ClientState, State},
    utils::surface::root_surface,
};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.wayland.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("unknown client data type")
            .compositor_client_state
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        layer_shell::shield_orphaned_layer_state(surface);
        drm_syncobj::hold_commits_until_ready(surface);
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

        if !is_sync_subsurface(surface)
            && let Some(window) = self.shell.window_for_surface(&root_surface(surface))
        {
            window.on_commit();
        }

        xdg_shell::handle_commit(self, surface);
        self.handle_layer_commit(surface);
        self.shell.advertise_scale(surface);
        self.queue_redraw_for_surface(surface);
    }

    fn destroyed(&mut self, _surface: &WlSurface) {}
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

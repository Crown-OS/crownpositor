use std::{os::unix::net::UnixStream, sync::Arc};

use smithay::{
    reexports::wayland_server::{
        DisplayHandle,
        backend::{ClientData, ClientId, DisconnectReason},
    },
    wayland::{compositor::CompositorClientState, security_context::SecurityContext},
};

#[derive(Default)]
pub struct ClientState {
    pub compositor_client_state: CompositorClientState,
    /// Set when the client connected through a `wp_security_context_v1`
    /// listener, i.e. a sandbox engine vouched for it and nothing more.
    pub security_context: Option<SecurityContext>,
}

impl ClientState {
    pub fn sandboxed(security_context: SecurityContext) -> Self {
        Self {
            security_context: Some(security_context),
            ..Self::default()
        }
    }

    /// A client that fails to insert has already hung up; there is nothing to
    /// keep and no reason to take the compositor down with it.
    pub fn insert(self, display_handle: &mut DisplayHandle, stream: UnixStream) {
        if let Err(err) = display_handle.insert_client(stream, Arc::new(self)) {
            tracing::warn!(%err, "failed to insert a wayland client");
        }
    }
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

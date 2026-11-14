use std::{os::unix::net::UnixStream, path::PathBuf, sync::Arc};

use smithay::{
    reexports::{
        rustix::net::sockopt::socket_peercred,
        wayland_server::{
            DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
        },
    },
    wayland::{compositor::CompositorClientState, security_context::SecurityContext},
};

#[derive(Default)]
pub struct ClientState {
    pub compositor_client_state: CompositorClientState,
    /// Set when the client connected through a `wp_security_context_v1`
    /// listener, i.e. a sandbox engine vouched for it and nothing more.
    pub security_context: Option<SecurityContext>,
    /// The program behind the connection, read from `/proc` as it connected.
    /// What `agents.ron` trusts is matched against this.
    pub executable: Option<PathBuf>,
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
    pub fn insert(mut self, display_handle: &mut DisplayHandle, stream: UnixStream) {
        self.executable = peer_executable(&stream);
        if let Err(err) = display_handle.insert_client(stream, Arc::new(self)) {
            tracing::warn!(%err, "failed to insert a wayland client");
        }
    }
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// The executable of the process at the other end of a freshly accepted
/// socket. Read before the client can speak, so it names the program that
/// connected and nothing it may exec later.
fn peer_executable(stream: &UnixStream) -> Option<PathBuf> {
    let pid = socket_peercred(stream).ok()?.pid;
    std::fs::read_link(format!("/proc/{}/exe", pid.as_raw_nonzero())).ok()
}

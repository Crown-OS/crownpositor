use std::{os::unix::net::UnixStream, path::PathBuf, sync::Arc};

use crownos_config::schema::AppKey;

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

use crate::utils::app_key::{PeerProcess, host_key, sandboxed_key};

#[derive(Default)]
pub struct ClientState {
    pub compositor_client_state: CompositorClientState,
    /// Set when the client connected through a `wp_security_context_v1`
    /// listener, i.e. a sandbox engine vouched for it and nothing more.
    pub security_context: Option<SecurityContext>,
    /// The program behind the connection, read from `/proc` as it connected.
    /// What `agents.ron` trusts is matched against this.
    pub executable: Option<PathBuf>,
    /// Who the client is for `context_access.ron`, resolved as it connected.
    /// `None` when it cannot be told, which gets no context.
    pub app_key: Option<AppKey>,
    /// The process that connected, unless the client is sandboxed.
    pub peer_process: Option<PeerProcess>,
}

impl ClientState {
    pub fn sandboxed(security_context: SecurityContext) -> Self {
        Self {
            app_key: sandboxed_key(&security_context),
            security_context: Some(security_context),
            ..Self::default()
        }
    }

    /// The key of the client, while the process that connected still runs.
    /// The program is not read again: a process may have turned off
    /// `PR_SET_DUMPABLE` since.
    pub fn current_app_key(&self) -> Option<&AppKey> {
        let unchanged = self.peer_process.as_ref().is_none_or(PeerProcess::is_alive);
        self.app_key.as_ref().filter(|_| unchanged)
    }

    /// A client that fails to insert has already hung up; there is nothing to
    /// keep and no reason to take the compositor down with it.
    pub fn insert(mut self, display_handle: &mut DisplayHandle, stream: UnixStream) {
        self.executable = peer_executable(&stream);
        if self.security_context.is_none()
            && let Some((key, process)) = host_key(&stream)
        {
            self.app_key = Some(key);
            self.peer_process = Some(process);
        }
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

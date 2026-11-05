use std::{os::unix::net::UnixStream, sync::Arc};

use protocols::crownos_android_shell::AndroidPeer;
use smithay::{
    reexports::wayland_server::{
        DisplayHandle,
        backend::{ClientData, ClientId, DisconnectReason},
    },
    wayland::{compositor::CompositorClientState, security_context::SecurityContext},
};

use crate::utils::client_identity::{self, KnownExecutable};

#[derive(Default)]
pub struct ClientState {
    pub compositor_client_state: CompositorClientState,
    /// Set when the client connected through a `wp_security_context_v1`
    /// listener, i.e. a sandbox engine vouched for it and nothing more.
    pub security_context: Option<SecurityContext>,
    /// Which of our programs an unsandboxed client runs, read as it
    /// connected. See [`crate::utils::client_identity`].
    pub executable: Option<KnownExecutable>,
    /// Which part of the Android runtime the client is, decided as it
    /// connected.
    pub android_peer: Option<AndroidPeer>,
}

impl ClientState {
    pub fn unsandboxed(executable: Option<KnownExecutable>) -> Self {
        Self {
            executable,
            android_peer: client_identity::unsandboxed_android_peer(executable),
            ..Self::default()
        }
    }

    pub fn sandboxed(security_context: SecurityContext, android_peer: Option<AndroidPeer>) -> Self {
        Self {
            security_context: Some(security_context),
            android_peer,
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

use protocols::crownos_android_shell::AndroidPeer;
use smithay::{
    reexports::wayland_server::DisplayHandle,
    wayland::security_context::{
        SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
    },
};

use crate::{
    state::{ClientState, State},
    utils::client_identity,
};

/// Every client accepted on a sandbox engine's listener is tagged with the
/// context it came through, which is what `utils::privilege` checks, and with
/// the part of the Android runtime it is, if the context is Waydroid's.
impl SecurityContextHandler for State {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        let inserted =
            self.common
                .event_loop_handle
                .insert_source(source, move |stream, _, state| {
                    let creator = creator_peer(&state.common.display_handle, &context);
                    let peer = client_identity::sandboxed_android_peer(
                        context.sandbox_engine.as_deref(),
                        creator,
                    );
                    ClientState::sandboxed(context.clone(), peer)
                        .insert(&mut state.common.display_handle, stream);
                });
        if let Err(err) = inserted {
            tracing::warn!(%err, "failed to listen on a security context");
        }
    }
}

/// What the client that created `context` is, while it is still connected
/// and unsandboxed.
fn creator_peer(display: &DisplayHandle, context: &SecurityContext) -> Option<AndroidPeer> {
    let data = display
        .backend_handle()
        .get_client_data(context.creator_client_id.clone())
        .ok()?;
    data.downcast_ref::<ClientState>()
        .filter(|creator| creator.security_context.is_none())
        .and_then(|creator| creator.android_peer)
}

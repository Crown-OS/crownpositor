use smithay::wayland::security_context::{
    SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
};

use crate::state::{ClientState, State};

/// Every client accepted on a sandbox engine's listener is tagged with the
/// context it came through, which is what `utils::privilege` checks.
impl SecurityContextHandler for State {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        let inserted =
            self.common
                .event_loop_handle
                .insert_source(source, move |stream, _, state| {
                    ClientState::sandboxed(context.clone())
                        .insert(&mut state.common.display_handle, stream);
                });
        if let Err(err) = inserted {
            tracing::warn!(%err, "failed to listen on a security context");
        }
    }
}

use smithay::reexports::wayland_server::Client;

use crate::state::ClientState;

/// Gates globals that must not reach sandboxed clients: reading or replacing
/// the clipboard in the background, blanking or reconfiguring outputs, locking
/// the session. A client is privileged unless it came in through a
/// `wp_security_context_v1` listener.
pub fn is_privileged(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|state| state.security_context.is_none())
}

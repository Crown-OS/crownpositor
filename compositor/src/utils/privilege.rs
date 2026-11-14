use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, RwLock},
};

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

/// Whether the client is one of the programs `agents.ron` trusts to drive the
/// desktop: its executable, read when it connected, is in `trusted`.
pub fn is_trusted_agent(client: &Client, trusted: &TrustedExecutables) -> bool {
    let Some(executable) = client
        .get_data::<ClientState>()
        .and_then(|state| state.executable.clone())
    else {
        return false;
    };
    trusted
        .read()
        .map(|set| set.contains(&executable))
        .unwrap_or(false)
}

/// The executables `agents.ron` trusts, shared with the global filters that
/// outlive any one config.
pub type TrustedExecutables = Arc<RwLock<HashSet<PathBuf>>>;

pub fn trusted_executables(paths: &[String]) -> HashSet<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

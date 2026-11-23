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

/// Whether the client can be named for `context_access.ron`. The context
/// global is shown only to such clients; the grant decides at bind.
pub fn has_app_key(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|state| state.app_key.is_some())
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

/// Whether the client is crownlink, the remote desktop's media engine, by the name of its
/// executable. A name is no proof, but every privileged client can inject into the user's own
/// seat through `crownos_input_v1`, which is more than a guest seat allows.
pub fn is_media_engine(client: &Client) -> bool {
    is_privileged(client)
        && client
            .get_data::<ClientState>()
            .and_then(|state| {
                state
                    .executable
                    .as_ref()?
                    .file_name()
                    .map(|name| name == MEDIA_ENGINE)
            })
            .unwrap_or(false)
}

const MEDIA_ENGINE: &str = "crownlink-linux";

/// The executables `agents.ron` trusts, shared with the global filters that
/// outlive any one config.
pub type TrustedExecutables = Arc<RwLock<HashSet<PathBuf>>>;

pub fn trusted_executables(paths: &[String]) -> HashSet<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

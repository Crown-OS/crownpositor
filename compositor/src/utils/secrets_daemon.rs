//! Which client is crownos-secretsd, the only one that may bind
//! `crownos_secret_input_manager_v1`.

use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use crownos_config::schema::AppKey;
use smithay::reexports::wayland_server::Client;

use crate::{state::ClientState, utils::development_override::development_path};

/// Where crownos-secrets-manager installs the daemon.
const SECRETS_DAEMON_EXECUTABLE: &str = "/usr/bin/crownos-secretsd";
/// A development build of the daemon to trust as well, for nested tests.
const DAEMON_OVERRIDE: &str = "CROWN_SECRETS_DAEMON";

static DEVELOPMENT_DAEMON: LazyLock<Option<PathBuf>> =
    LazyLock::new(|| development_path(DAEMON_OVERRIDE));

/// The installed daemon, while root controls it, still the process that
/// connected and without preloaded code.
pub fn is_secrets_daemon(client: &Client) -> bool {
    client.get_data::<ClientState>().is_some_and(|state| {
        state.security_context.is_none()
            && !state.is_preloaded()
            && state.current_app_key().is_some()
            && state
                .verified_executable()
                .is_some_and(is_secrets_daemon_executable)
    })
}

fn is_secrets_daemon_executable(executable: &Path) -> bool {
    if DEVELOPMENT_DAEMON.as_deref() == Some(executable) {
        return true;
    }
    let installed = AppKey::HostExecutable {
        path: executable.to_path_buf(),
    };
    executable == Path::new(SECRETS_DAEMON_EXECUTABLE) && !installed.is_session_only(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_installed_daemon_is_trusted() {
        for path in [
            "/usr/bin/crownos-secrets",
            "/home/user/.local/bin/crownos-secretsd",
            "/tmp/crownos-secretsd",
            "/usr/bin/crownos-computer-use",
        ] {
            assert!(!is_secrets_daemon_executable(Path::new(path)), "{path}");
        }
        let installed = Path::new(SECRETS_DAEMON_EXECUTABLE);
        let root_controls_it = !AppKey::HostExecutable {
            path: installed.to_path_buf(),
        }
        .is_session_only(None);
        assert_eq!(is_secrets_daemon_executable(installed), root_controls_it);
    }
}

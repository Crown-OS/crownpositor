//! Who a client is, decided once as it connects: by the executable of the
//! process that connected, and for a sandboxed client by who created its
//! security context. Global filters run while the display is busy and
//! cannot ask it anything, so the answer has to be on the client's data by
//! then.

use std::{os::fd::AsFd, path::Path};

use protocols::crownos_android_shell::AndroidPeer;
use smithay::reexports::wayland_server::Client;

use crate::state::ClientState;

/// What the kernel appends to `/proc/<pid>/exe` once the file is replaced, as
/// a package upgrade does under a running program.
const REPLACED_SUFFIX: &str = " (deleted)";
/// The sandbox engine Waydroid's security context is created with.
const WAYDROID_SANDBOX_ENGINE: &str = "id.waydro.waydroid";

/// The CrownOS programs the compositor treats differently, known by the file
/// name of their executable, so a development build anywhere on disk counts
/// too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownExecutable {
    /// `crown-androidd`, the Android runtime daemon.
    AndroidDaemon,
    /// `crownotify`, the notification centre.
    Notifications,
}

impl KnownExecutable {
    const ALL: [Self; 2] = [Self::AndroidDaemon, Self::Notifications];

    fn file_name(self) -> &'static str {
        match self {
            Self::AndroidDaemon => "crown-androidd",
            Self::Notifications => "crownotify",
        }
    }

    /// The program at `executable`, as `/proc/<pid>/exe` names it.
    fn of_path(executable: &Path) -> Option<Self> {
        let name = executable.file_name()?.to_str()?;
        let name = name.strip_suffix(REPLACED_SUFFIX).unwrap_or(name);
        Self::ALL
            .into_iter()
            .find(|known| known.file_name() == name)
    }
}

/// The program on the other end of `stream`, if it is one of ours.
pub fn executable_of(stream: impl AsFd) -> Option<KnownExecutable> {
    let peer = rustix::net::sockopt::socket_peercred(stream).ok()?;
    let executable = std::fs::read_link(format!("/proc/{}/exe", peer.pid.as_raw_pid())).ok()?;
    KnownExecutable::of_path(&executable)
}

/// An unsandboxed client is the Android daemon when it runs the daemon.
pub fn unsandboxed_android_peer(executable: Option<KnownExecutable>) -> Option<AndroidPeer> {
    (executable == Some(KnownExecutable::AndroidDaemon)).then_some(AndroidPeer::Daemon)
}

/// A client from a security context is the hwcomposer when the context is
/// Waydroid's and `creator` — the live, unsandboxed client that created it —
/// is the daemon.
pub fn sandboxed_android_peer(
    sandbox_engine: Option<&str>,
    creator: Option<AndroidPeer>,
) -> Option<AndroidPeer> {
    (sandbox_engine == Some(WAYDROID_SANDBOX_ENGINE) && creator == Some(AndroidPeer::Daemon))
        .then_some(AndroidPeer::Hwcomposer)
}

/// Which part of the CrownOS Android runtime `client` is, if any. Only those
/// see `crownos_android_shell_v1`.
pub fn android_peer(client: &Client) -> Option<AndroidPeer> {
    client
        .get_data::<ClientState>()
        .and_then(|state| state.android_peer)
}

/// Which of our programs `client` is, if it is one and unsandboxed.
pub fn executable(client: &Client) -> Option<KnownExecutable> {
    client
        .get_data::<ClientState>()
        .and_then(|state| state.executable)
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use super::*;

    fn known(path: &str) -> Option<KnownExecutable> {
        KnownExecutable::of_path(Path::new(path))
    }

    #[test]
    fn programs_are_known_by_their_file_name() {
        for path in [
            "/usr/bin/crown-androidd",
            "/home/dev/crownos-android/target/debug/crown-androidd",
            "/usr/bin/crown-androidd (deleted)",
        ] {
            assert_eq!(known(path), Some(KnownExecutable::AndroidDaemon), "{path}");
        }
        assert_eq!(
            known("/usr/bin/crownotify"),
            Some(KnownExecutable::Notifications)
        );
    }

    #[test]
    fn other_executables_are_unknown() {
        for path in [
            "/usr/bin/crown-android",
            "/usr/bin/crown-androidd-helper",
            "/usr/bin/crownotify-send",
            "/usr/bin/waydroid",
            "/usr/lib/crown-androidd/helper",
            "/",
        ] {
            assert_eq!(known(path), None, "{path}");
        }
    }

    #[test]
    fn only_the_daemon_is_an_unsandboxed_android_peer() {
        assert_eq!(
            unsandboxed_android_peer(Some(KnownExecutable::AndroidDaemon)),
            Some(AndroidPeer::Daemon)
        );
        assert_eq!(
            unsandboxed_android_peer(Some(KnownExecutable::Notifications)),
            None
        );
        assert_eq!(unsandboxed_android_peer(None), None);
    }

    #[test]
    fn only_waydroids_context_from_the_daemon_is_the_hwcomposer() {
        let daemon = Some(AndroidPeer::Daemon);
        assert_eq!(
            sandboxed_android_peer(Some(WAYDROID_SANDBOX_ENGINE), daemon),
            Some(AndroidPeer::Hwcomposer)
        );
        assert_eq!(
            sandboxed_android_peer(Some(WAYDROID_SANDBOX_ENGINE), None),
            None
        );
        assert_eq!(
            sandboxed_android_peer(Some(WAYDROID_SANDBOX_ENGINE), Some(AndroidPeer::Hwcomposer)),
            None,
            "a sandboxed creator vouches for nothing"
        );
        assert_eq!(sandboxed_android_peer(Some("org.flatpak"), daemon), None);
        assert_eq!(sandboxed_android_peer(None, daemon), None);
    }

    #[test]
    fn a_process_running_something_else_is_unknown() {
        let (ours, _theirs) = UnixStream::pair().expect("socket pair");
        assert_eq!(executable_of(&ours), None);
    }
}

//! Which application is behind a Wayland client, in the form
//! `context_access.ron` stores grants under.
//!
//! A sandboxed client is named by its `wp_security_context_v1`. Any other
//! client is named by the program of the process that connected, read once
//! as it connects through [`crownos_ipc::Credentials`]: its pidfd, and the
//! executable whose device and inode match the running one. A process that
//! turned off `PR_SET_DUMPABLE` before it connected cannot be read and gets
//! no key.

use std::os::{fd::AsRawFd, fd::OwnedFd, unix::net::UnixStream};

use crownos_config::schema::AppKey;
use crownos_ipc::Credentials;
use smithay::wayland::security_context::SecurityContext;

/// The process that connected, kept to check at bind that it still runs.
#[derive(Debug)]
pub struct PeerProcess {
    pid: i32,
    pidfd: OwnedFd,
}

impl PeerProcess {
    /// Whether the process that connected still runs. Its pidfd names that
    /// process only, so a live pidfd means the same process.
    pub fn is_alive(&self) -> bool {
        std::fs::read_to_string(format!("/proc/self/fdinfo/{}", self.pidfd.as_raw_fd()))
            .ok()
            .and_then(|fdinfo| {
                fdinfo
                    .lines()
                    .find_map(|line| line.strip_prefix("Pid:"))
                    .and_then(|pid| pid.trim().parse::<i32>().ok())
            })
            .is_some_and(|pid| pid > 0 && pid == self.pid)
    }
}

/// The key of a sandboxed client, when the sandbox engine named both itself
/// and the app.
pub fn sandboxed_key(context: &SecurityContext) -> Option<AppKey> {
    let engine = context
        .sandbox_engine
        .as_deref()
        .filter(|text| !text.is_empty())?;
    let app_id = context.app_id.as_deref().filter(|text| !text.is_empty())?;
    Some(AppKey::Sandboxed {
        engine: engine.to_owned(),
        app_id: app_id.to_owned(),
    })
}

/// The key and the process of an unsandboxed client, read before the client
/// can speak.
pub fn host_key(stream: &UnixStream) -> Option<(AppKey, PeerProcess)> {
    let credentials = Credentials::from_socket(stream).ok()?;
    let program = credentials.program.as_ref()?;
    let key = AppKey::of_program(
        &program.executable,
        &program.arguments,
        program.working_directory.as_deref(),
    )?;
    let process = PeerProcess {
        pid: credentials.pid,
        pidfd: credentials.pidfd?,
    };
    Some((key, process))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_named_by_its_executable() -> std::io::Result<()> {
        let (ours, _theirs) = UnixStream::pair()?;
        let (key, process) = host_key(&ours).ok_or(std::io::ErrorKind::NotFound)?;
        let executable = std::env::current_exe()?;
        assert_eq!(key, AppKey::HostExecutable { path: executable });
        assert!(process.is_alive());
        Ok(())
    }
}

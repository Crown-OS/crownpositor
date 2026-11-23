//! Which application is behind a Wayland client, in the form
//! `context_access.ron` stores grants under.
//!
//! A sandboxed client is named by its `wp_security_context_v1`. Any other
//! client is named by the program of the process that connected, read once
//! as it connects through [`crownos_ipc::Credentials`]: its pidfd, and the
//! executable whose device and inode match the running one. A process that
//! turned off `PR_SET_DUMPABLE` before it connected cannot be read and gets
//! no key.

use std::{
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::{Path, PathBuf},
};

use crownos_config::schema::AppKey;
use crownos_ipc::{Credentials, pidfd_names_pid};
use smithay::wayland::security_context::SecurityContext;

/// Environment variables that load other code into a program.
const PRELOAD_VARIABLES: [&[u8]; 2] = [b"LD_PRELOAD=", b"LD_LIBRARY_PATH="];

/// The process that connected, kept to check at bind that it still runs.
#[derive(Debug)]
pub struct PeerProcess {
    pid: i32,
    pidfd: OwnedFd,
    /// The executable it ran when it connected, checked by device and inode.
    executable: PathBuf,
    /// It started with `LD_PRELOAD` or `LD_LIBRARY_PATH`, so it may not run
    /// the code its executable holds.
    preloaded: bool,
}

impl PeerProcess {
    /// Whether the process that connected still runs. Its pidfd names that
    /// process only, so a live pidfd means the same process.
    pub fn is_alive(&self) -> bool {
        pidfd_names_pid(&self.pidfd, self.pid)
    }

    pub const fn pid(&self) -> i32 {
        self.pid
    }

    pub const fn is_preloaded(&self) -> bool {
        self.preloaded
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }
}

/// Whether the environment of `pid` loads other code. An environment that
/// cannot be read counts as loading other code.
fn is_preloaded(pid: i32) -> bool {
    std::fs::read(format!("/proc/{pid}/environ")).map_or(true, |environment| {
        environment.split(|byte| *byte == 0).any(|variable| {
            PRELOAD_VARIABLES
                .iter()
                .any(|preload| variable.starts_with(preload))
        })
    })
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
        executable: program.executable.clone(),
        pid: credentials.pid,
        preloaded: is_preloaded(credentials.pid),
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

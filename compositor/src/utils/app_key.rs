//! Which application is behind a Wayland client, in the form
//! `context_access.ron` stores grants under.
//!
//! A sandboxed client is named by its `wp_security_context_v1`. Any other
//! client is named by the executable of the process that connected: the
//! kernel gives its pidfd (`SO_PEERPIDFD`), so a later process that reuses
//! the pid cannot pass for it, and the device and inode of `/proc/<pid>/exe`
//! must match the file at the path, so a replaced file does not pass. A
//! known interpreter is named with its script.

use std::{
    io,
    mem::size_of,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
};

use crownos_config::schema::AppKey;
use smithay::{
    reexports::rustix::net::sockopt::socket_peercred, wayland::security_context::SecurityContext,
};

/// Interpreters whose clients are named by their script, by file name prefix.
const INTERPRETERS: [&str; 12] = [
    "python", "node", "nodejs", "bun", "deno", "perl", "ruby", "bash", "sh", "dash", "zsh", "lua",
];
/// Words between an interpreter and its script that are not the script.
const INTERPRETER_SUBCOMMANDS: [&str; 1] = ["run"];

/// The process that connected, kept to check at bind that it did not exec.
#[derive(Debug)]
pub struct PeerProcess {
    pid: i32,
    pidfd: OwnedFd,
    executable: (u64, u64),
}

impl PeerProcess {
    /// Whether the process still runs the executable it connected with.
    pub fn runs_same_executable(&self) -> bool {
        executable_inode(self.pid).ok() == Some(self.executable) && self.is_alive()
    }

    fn is_alive(&self) -> bool {
        pid_of_pidfd(&self.pidfd) == Some(self.pid)
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
    let pid = socket_peercred(stream).ok()?.pid.as_raw_nonzero().get();
    let pidfd = peer_pidfd(stream).ok()?;
    let proc_exe = PathBuf::from(format!("/proc/{pid}/exe"));
    let executable_path = std::fs::read_link(&proc_exe).ok()?;
    let executable = executable_inode(pid).ok()?;
    let on_disk = std::fs::metadata(&executable_path).ok()?;
    if (on_disk.dev(), on_disk.ino()) != executable {
        return None;
    }
    let key = if is_interpreter(&executable_path) {
        AppKey::Interpreted {
            script: script_of(pid)?,
            interpreter: executable_path,
        }
    } else {
        AppKey::HostExecutable {
            path: executable_path,
        }
    };
    let process = PeerProcess {
        pid,
        pidfd,
        executable,
    };
    process.is_alive().then_some((key, process))
}

fn executable_inode(pid: i32) -> io::Result<(u64, u64)> {
    let metadata = std::fs::metadata(format!("/proc/{pid}/exe"))?;
    Ok((metadata.dev(), metadata.ino()))
}

fn is_interpreter(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            INTERPRETERS.iter().any(|interpreter| {
                name.strip_prefix(interpreter).is_some_and(|version| {
                    version
                        .chars()
                        .all(|character| character.is_ascii_digit() || character == '.')
                })
            })
        })
}

/// The first argument that names a script, made absolute against the
/// working directory of the process. `None` for `python -c` and the like.
fn script_of(pid: i32) -> Option<PathBuf> {
    let command_line = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let script = command_line
        .split(|byte| *byte == 0)
        .skip(1)
        .filter_map(|argument| std::str::from_utf8(argument).ok())
        .find(|argument| {
            !argument.is_empty()
                && !argument.starts_with('-')
                && !INTERPRETER_SUBCOMMANDS.contains(argument)
        })?;
    let script = PathBuf::from(script);
    if script.is_absolute() {
        return Some(script);
    }
    let working_directory = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    Some(working_directory.join(script))
}

fn peer_pidfd(stream: &UnixStream) -> io::Result<OwnedFd> {
    let mut raw: libc::c_int = -1;
    let mut length = size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `raw` and `length` are valid for writes and describe a c_int.
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERPIDFD,
            (&raw mut raw).cast(),
            &mut length,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    if raw < 0 {
        return Err(io::ErrorKind::NotFound.into());
    }
    // SAFETY: the kernel installed `raw` for this process and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// The pid a pidfd names in this pid namespace, or `None` once it exited.
fn pid_of_pidfd(pidfd: &OwnedFd) -> Option<i32> {
    let fdinfo =
        std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd())).ok()?;
    fdinfo
        .lines()
        .find_map(|line| line.strip_prefix("Pid:"))
        .and_then(|pid| pid.trim().parse().ok())
        .filter(|pid: &i32| *pid > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpreters_are_known_by_name_and_version() {
        for interpreter in [
            "/usr/bin/python3",
            "/usr/bin/python3.13",
            "/usr/bin/node",
            "/bin/sh",
        ] {
            assert!(is_interpreter(Path::new(interpreter)), "{interpreter}");
        }
        for program in [
            "/usr/bin/shotwell",
            "/usr/bin/nodecraft",
            "/usr/bin/crown-contextd",
        ] {
            assert!(!is_interpreter(Path::new(program)), "{program}");
        }
    }

    #[test]
    fn this_process_is_named_by_its_executable() -> io::Result<()> {
        let (ours, _theirs) = UnixStream::pair()?;
        let (key, process) = host_key(&ours).ok_or(io::ErrorKind::NotFound)?;
        let executable = std::env::current_exe()?;
        assert_eq!(key, AppKey::HostExecutable { path: executable });
        assert!(process.runs_same_executable());
        Ok(())
    }
}

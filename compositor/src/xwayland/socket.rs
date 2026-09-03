//! The X11 display slot.
//!
//! An X server is addressed by a display number, and the convention that makes
//! `DISPLAY=:3` mean anything is a filesystem one: a lock file nobody else may
//! create and sockets everybody may connect to. The compositor takes the slot
//! and holds the listening sockets itself rather than letting an X server do
//! it, so that a client can connect before anything has been started — that
//! connection is what starts it.
//!
//! The directory checks come from Mutter by way of niri. `/tmp/.X11-unix` is
//! world-writable by design, so a slot in a directory somebody else created
//! with the wrong ownership is a slot somebody else can impersonate.

use std::{
    fs::File,
    io::Write,
    os::{
        linux::net::SocketAddrExt,
        unix::{
            ffi::OsStrExt,
            net::{SocketAddr, UnixListener},
        },
    },
    path::{Path, PathBuf},
};

use smithay::reexports::rustix::{
    self,
    fs::{Mode, OFlags},
    io::Errno,
    process::{Pid, getpid, getuid, test_kill_process},
};

const TMP: &str = "/tmp";
const SOCKET_DIR: &str = "/tmp/.X11-unix";
/// Xorg gives up after this many, and so do we.
const ATTEMPTS: u32 = 50;

#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    #[error("{SOCKET_DIR} could not be created")]
    Directory(#[source] Errno),
    #[error("{SOCKET_DIR} exists but is not safe to use")]
    Permissions,
    #[error("no free X11 display between :{first} and :{last}")]
    Exhausted { first: u32, last: u32 },
}

/// One X11 display number, with its lock file and listening sockets held for
/// as long as this value lives.
#[derive(Debug)]
pub struct Sockets {
    name: String,
    listeners: Vec<UnixListener>,
    _cleanup: Vec<Unlink>,
}

impl Sockets {
    pub fn bind() -> Result<Self, SocketError> {
        Self::bind_from(0)
    }

    /// The display number, in the form `DISPLAY` wants it.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn listeners(&self) -> &[UnixListener] {
        &self.listeners
    }

    /// `start` is a parameter so the tests can take a slot no real X server
    /// would be given.
    fn bind_from(start: u32) -> Result<Self, SocketError> {
        ensure_socket_dir()?;

        for number in start..start + ATTEMPTS {
            let Some(lock) = take_lock(number) else {
                continue;
            };

            match listen(number) {
                Ok((listeners, socket)) => {
                    return Ok(Self {
                        name: format!(":{number}"),
                        listeners,
                        _cleanup: vec![lock, socket],
                    });
                }
                // Dropping `lock` frees the number for whoever can use it.
                Err(err) => tracing::debug!(display = number, %err, "X11 display is unusable"),
            }
        }

        Err(SocketError::Exhausted {
            first: start,
            last: start + ATTEMPTS - 1,
        })
    }
}

/// Removes a path once nothing needs it. Both the lock file and the socket are
/// stale the moment the compositor exits, and a stale lock file costs the next
/// compositor a display number.
#[derive(Debug)]
struct Unlink(PathBuf);

impl Drop for Unlink {
    fn drop(&mut self) {
        if let Err(err) = rustix::fs::unlink(&self.0) {
            tracing::warn!(path = %self.0.display(), %err, "failed to remove an X11 socket file");
        }
    }
}

fn ensure_socket_dir() -> Result<(), SocketError> {
    match rustix::fs::mkdir(SOCKET_DIR, Mode::from(0o1777)) {
        Ok(()) => Ok(()),
        Err(Errno::EXIST) => check_socket_dir(),
        Err(err) => Err(SocketError::Directory(err)),
    }
}

/// A directory somebody else owns, or that anyone may delete out of, is one
/// where our socket can be swapped for theirs.
fn check_socket_dir() -> Result<(), SocketError> {
    let socket_dir = rustix::fs::lstat(SOCKET_DIR).map_err(SocketError::Directory)?;
    let tmp = rustix::fs::lstat(TMP).map_err(SocketError::Directory)?;

    let owned = socket_dir.st_uid == tmp.st_uid || socket_dir.st_uid == getuid().as_raw();
    let writable = socket_dir.st_mode & 0o022 == 0o022;
    let sticky = socket_dir.st_mode & 0o1000 == 0o1000;

    (owned && writable && sticky)
        .then_some(())
        .ok_or(SocketError::Permissions)
}

fn take_lock(display: u32) -> Option<Unlink> {
    let path = lock_path(display);

    match create_lock(&path) {
        Some(lock) => Some(lock),
        None if reclaim_lock(&path) => create_lock(&path),
        None => None,
    }
}

/// `O_EXCL` is the whole lock: whoever creates the file owns the number.
fn create_lock(path: &Path) -> Option<Unlink> {
    let flags = OFlags::WRONLY | OFlags::CLOEXEC | OFlags::CREATE | OFlags::EXCL;
    let file = rustix::fs::open(path, flags, Mode::from(0o444)).ok()?;
    let guard = Unlink(path.to_owned());

    File::from(file)
        .write_all(lock_contents(getpid().as_raw_nonzero().get()).as_bytes())
        .ok()?;

    Some(guard)
}

/// A server killed with `SIGKILL` leaves its lock file behind, and without
/// this every such crash would cost a display number until the next reboot.
/// Xorg reclaims these the same way: if the recorded process is gone, so is
/// its claim.
fn reclaim_lock(path: &Path) -> bool {
    let holder = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.trim().parse().ok())
        .and_then(Pid::from_raw);

    // Unreadable or unparseable means no owner can be established, and a live
    // process means the claim stands. `EPERM` says it is alive but not ours.
    match holder {
        Some(pid) if test_kill_process(pid) != Err(Errno::SRCH) => false,
        Some(_) | None => {
            tracing::debug!(path = %path.display(), "reclaiming a stale X11 lock file");
            rustix::fs::unlink(path).is_ok()
        }
    }
}

/// Both sockets carry the same name; a client connects to whichever its libX11
/// was built for, so binding only one silently excludes half of them.
fn listen(display: u32) -> std::io::Result<(Vec<UnixListener>, Unlink)> {
    let path = socket_path(display);

    // The number is ours, so a leftover from a crashed server is not a reason
    // to fail.
    let _ = rustix::fs::unlink(&path);
    let bound = UnixListener::bind(&path)?;
    let cleanup = Unlink(path.clone());

    let abstract_address = SocketAddr::from_abstract_name(path.as_os_str().as_bytes())?;
    let listeners = vec![bound, UnixListener::bind_addr(&abstract_address)?];

    Ok((listeners, cleanup))
}

fn lock_path(display: u32) -> PathBuf {
    PathBuf::from(format!("{TMP}/.X{display}-lock"))
}

fn socket_path(display: u32) -> PathBuf {
    PathBuf::from(format!("{SOCKET_DIR}/X{display}"))
}

/// Ten right-aligned columns and a newline, which is what every reader of
/// these files expects.
fn lock_contents(pid: i32) -> String {
    format!("{pid:>10}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Far above anything a real X server would take, and spaced by more than
    /// `ATTEMPTS` so that two tests running at once cannot scan into each
    /// other's numbers and reclaim a slot the other still holds.
    const CLAIM: u32 = 700;
    const SECOND: u32 = CLAIM + ATTEMPTS + 10;
    const RECLAIM: u32 = SECOND + ATTEMPTS + 10;

    #[test]
    fn a_lock_file_is_ten_columns_and_a_newline() {
        assert_eq!(lock_contents(1234), "      1234\n");
        assert_eq!(lock_contents(1).len(), 11);
        assert_eq!(lock_contents(1234567890).len(), 11);
    }

    #[test]
    fn paths_follow_the_x11_convention() {
        assert_eq!(lock_path(3), Path::new("/tmp/.X3-lock"));
        assert_eq!(socket_path(3), Path::new("/tmp/.X11-unix/X3"));
    }

    #[test]
    fn binding_claims_a_slot_and_gives_it_back() {
        let Ok(sockets) = Sockets::bind_from(CLAIM) else {
            // A sandbox without a writable /tmp is not a failing test.
            return;
        };

        let display: u32 = sockets.name()[1..].parse().expect("a numeric display");
        assert!(lock_path(display).exists());
        assert!(socket_path(display).exists());
        assert_eq!(sockets.listeners().len(), 2);

        drop(sockets);
        assert!(!lock_path(display).exists());
        assert!(!socket_path(display).exists());
    }

    #[test]
    fn a_second_binding_takes_a_different_slot() {
        let Ok(first) = Sockets::bind_from(SECOND) else {
            return;
        };
        let second = Sockets::bind_from(SECOND).expect("the next number is free");

        assert_ne!(first.name(), second.name());
    }

    #[test]
    fn a_lock_file_whose_process_is_gone_is_reclaimed() {
        let path = lock_path(RECLAIM);
        let _ = rustix::fs::unlink(&path);

        // pid 1 is always alive; a lock file naming it must be left alone.
        if std::fs::write(&path, lock_contents(1)).is_err() {
            return;
        }
        assert!(!reclaim_lock(&path), "a live holder keeps its claim");

        // No process can hold the reserved pid 0, so this one is junk.
        std::fs::write(&path, lock_contents(0)).expect("a writable scratch path");
        assert!(reclaim_lock(&path));
        assert!(!path.exists());
    }
}

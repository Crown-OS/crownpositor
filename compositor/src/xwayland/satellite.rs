//! The bridge process.
//!
//! `xwayland-satellite` runs Xwayland out of process and re-presents X11
//! windows to us as ordinary `xdg_toplevel`s, so the compositor needs no X
//! window manager of its own. It is handed the listening sockets rather than
//! a display number to bind, which is what lets the compositor own the display
//! slot for its whole life and start the bridge only when it is wanted.
//!
//! Nothing here knows about the event loop: the caller supplies what to do
//! when the process is gone.

use std::{
    ffi::OsStr,
    io,
    os::{
        fd::{AsRawFd, BorrowedFd, RawFd},
        unix::{net::UnixListener, process::CommandExt},
    },
    process::{Command, ExitStatus, Stdio},
    thread,
};

use smithay::reexports::rustix::io::{FdFlags, fcntl_setfd};

/// Older releases bind the display themselves and would fight us for the slot.
const LISTENFD_PROBE: &str = "--test-listenfd-support";

/// Whether this binary exists, runs, and can take listening sockets from us.
pub fn probe(binary: &OsStr) -> bool {
    Command::new(binary)
        .arg(":0")
        .arg(LISTENFD_PROBE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("DISPLAY")
        .status()
        .is_ok_and(|status| status.success())
}

/// Starts the bridge and calls `finished` once it is gone, whether it failed
/// to start or ran and exited.
///
/// Reaping happens on its own thread because `wait` blocks until the process
/// ends, which is for as long as an X11 client is alive.
pub fn supervise(
    binary: &OsStr,
    display: &str,
    listeners: &[UnixListener],
    finished: impl FnOnce(io::Result<ExitStatus>) + Send + 'static,
) -> io::Result<()> {
    let listeners = listeners
        .iter()
        .map(UnixListener::try_clone)
        .collect::<io::Result<Vec<_>>>()?;
    let command = command(binary, display, &listeners);

    thread::Builder::new()
        .name(String::from("crown-xwayland"))
        .spawn(move || finished(run(command, listeners)))
        .map(drop)
}

fn run(mut command: Command, listeners: Vec<UnixListener>) -> io::Result<ExitStatus> {
    let mut child = command.spawn()?;
    // The child holds its own copies now, and ours would keep the sockets
    // referenced past the point anything reads them.
    drop(listeners);
    child.wait()
}

fn command(binary: &OsStr, display: &str, listeners: &[UnixListener]) -> Command {
    let fds: Vec<RawFd> = listeners.iter().map(AsRawFd::as_raw_fd).collect();

    let mut command = Command::new(binary);
    command
        .args(argv(display, &fds))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Ours points at the sockets this process is about to serve.
        .env_remove("DISPLAY");

    // Safety: `fcntl` is async-signal-safe and the only call between fork and
    // exec.
    unsafe {
        command.pre_exec(move || {
            for fd in &fds {
                // Safety: the parent holds these open until `spawn` returns.
                let fd = BorrowedFd::borrow_raw(*fd);
                fcntl_setfd(fd, FdFlags::empty())?;
            }
            Ok(())
        });
    }

    command
}

fn argv(display: &str, listeners: &[RawFd]) -> Vec<String> {
    let mut argv = Vec::with_capacity(1 + listeners.len() * 2);
    argv.push(display.to_owned());

    for fd in listeners {
        argv.push(String::from("-listenfd"));
        argv.push(fd.to_string());
    }

    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listener_is_passed_on() {
        assert_eq!(
            argv(":3", &[7, 9]),
            [":3", "-listenfd", "7", "-listenfd", "9"]
        );
    }

    #[test]
    fn a_display_with_no_listeners_is_still_well_formed() {
        assert_eq!(argv(":0", &[]), [":0"]);
    }

    #[test]
    fn a_missing_binary_does_not_probe_true() {
        assert!(!probe(OsStr::new("/nonexistent/xwayland-satellite")));
    }

    #[test]
    fn a_binary_that_rejects_the_probe_does_not_probe_true() {
        assert!(!probe(OsStr::new("/bin/false")));
    }
}

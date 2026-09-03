//! When the bridge starts.
//!
//! Xwayland is not free — a process, a socket and a few megabytes — and most
//! sessions never run an X11 client at all. So the compositor holds the
//! display slot from the moment it starts, advertises it through `DISPLAY`,
//! and starts nothing until something actually connects. The connection that
//! triggers the start is served by the bridge, so a client never notices the
//! wait.
//!
//! The bridge is restarted the same way it was started: it exits when the last
//! X11 client goes away, and the next client brings it back.

use std::{
    borrow::Cow,
    ffi::{OsStr, OsString},
    io, mem,
    os::unix::net::UnixListener,
    process::ExitStatus,
};

use calloop::{
    Interest, LoopHandle, Mode, PostAction, RegistrationToken, channel, generic::Generic,
};

use crate::{
    state::State,
    xwayland::{satellite, socket::Sockets},
};

const BINARY: &str = "xwayland-satellite";
const BINARY_OVERRIDE: &str = "CROWN_XWAYLAND_SATELLITE";

/// The compositor's X11 support, or the absence of it.
pub struct Xwayland(Option<Active>);

struct Active {
    loop_handle: LoopHandle<'static, State>,
    sockets: Sockets,
    binary: Cow<'static, OsStr>,
    exits: channel::Sender<io::Result<ExitStatus>>,
    activation: Activation,
}

enum Activation {
    Listening(Vec<RegistrationToken>),
    Running,
}

impl Xwayland {
    /// Claims a display and arms the sockets. Every failure here leaves the
    /// session working without X11 rather than not working at all.
    pub fn start(loop_handle: &LoopHandle<'static, State>) -> Self {
        let binary = binary(std::env::var_os(BINARY_OVERRIDE));
        if !satellite::probe(&binary) {
            tracing::info!(
                binary = ?binary,
                "no usable X11 bridge; X11 clients are not supported this session"
            );
            return Self(None);
        }

        let sockets = match Sockets::bind() {
            Ok(sockets) => sockets,
            Err(err) => {
                tracing::warn!(%err, "X11 clients are not supported this session");
                return Self(None);
            }
        };

        let (exits, receiver) = channel::channel();
        let watching = loop_handle.insert_source(receiver, |event, _, state| {
            if let channel::Event::Msg(outcome) = event {
                state.xwayland.restart(outcome);
            }
        });
        if let Err(err) = watching {
            tracing::warn!(%err, "X11 clients are not supported this session");
            return Self(None);
        }

        tracing::info!(display = sockets.name(), "listening for X11 clients");

        let active = Active {
            activation: Activation::Listening(arm(loop_handle, &sockets)),
            loop_handle: loop_handle.clone(),
            sockets,
            binary,
            exits,
        };

        Self(Some(active))
    }

    /// What children should be given as `DISPLAY`, if anything.
    pub fn display_name(&self) -> Option<&str> {
        self.0.as_ref().map(|active| active.sockets.name())
    }

    fn activate(&mut self) {
        let Some(active) = &mut self.0 else {
            return;
        };

        if active.disarm() {
            active.launch();
        }
    }

    fn restart(&mut self, outcome: io::Result<ExitStatus>) {
        let Some(active) = &mut self.0 else {
            return;
        };

        match outcome {
            Ok(status) => {
                tracing::info!(binary = ?active.binary, %status, "the X11 bridge exited")
            }
            Err(err) => tracing::warn!(binary = ?active.binary, %err, "the X11 bridge failed"),
        }

        active.listen();
    }
}

impl Active {
    fn listen(&mut self) {
        self.disarm();
        self.activation = Activation::Listening(arm(&self.loop_handle, &self.sockets));
    }

    /// Whether it was listening, so a second connection arriving alongside the
    /// first cannot start a second bridge.
    fn disarm(&mut self) -> bool {
        let Activation::Listening(tokens) = mem::replace(&mut self.activation, Activation::Running)
        else {
            return false;
        };

        for token in tokens {
            self.loop_handle.remove(token);
        }

        true
    }

    fn launch(&mut self) {
        let exits = self.exits.clone();
        let started = satellite::supervise(
            &self.binary,
            self.sockets.name(),
            self.sockets.listeners(),
            move |outcome| {
                // A closed channel means the compositor is shutting down and
                // has no use for the news.
                let _ = exits.send(outcome);
            },
        );

        match started {
            Ok(()) => tracing::info!(
                binary = ?self.binary,
                display = self.sockets.name(),
                "started the X11 bridge"
            ),
            Err(err) => {
                tracing::warn!(binary = ?self.binary, %err, "failed to start the X11 bridge");
                self.listen();
            }
        }
    }
}

fn arm(loop_handle: &LoopHandle<'static, State>, sockets: &Sockets) -> Vec<RegistrationToken> {
    sockets
        .listeners()
        .iter()
        .filter_map(|listener| {
            let source = watch(listener)
                .inspect_err(|err| tracing::warn!(%err, "failed to prepare an X11 socket"))
                .ok()?;

            loop_handle
                .insert_source(source, |_, _, state| {
                    state.xwayland.activate();
                    Ok(PostAction::Remove)
                })
                .inspect_err(|err| tracing::warn!(%err, "failed to watch an X11 socket"))
                .ok()
        })
        .collect()
}

fn watch(listener: &UnixListener) -> io::Result<Generic<UnixListener>> {
    let listener = listener.try_clone()?;
    drain(&listener)?;

    Ok(Generic::new(listener, Interest::READ, Mode::Level))
}

/// Nothing on this side ever accepts — the bridge does — so a connection left
/// queued from a bridge that failed to start would wake the compositor for as
/// long as it stayed queued. Whoever was waiting has already lost their
/// connection by the time we get here.
fn drain(listener: &UnixListener) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    while listener.accept().is_ok() {}
    listener.set_nonblocking(false)
}

fn binary(configured: Option<OsString>) -> Cow<'static, OsStr> {
    configured.map_or(Cow::Borrowed(OsStr::new(BINARY)), Cow::Owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_name_is_used_by_default() {
        assert_eq!(binary(None), Cow::Borrowed(OsStr::new(BINARY)));
    }

    #[test]
    fn an_override_is_taken_verbatim() {
        let configured = OsString::from("/opt/xwls");
        assert_eq!(
            binary(Some(configured.clone())),
            Cow::<OsStr>::Owned(configured)
        );
    }
}

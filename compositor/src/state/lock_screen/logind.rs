//! logind: `loginctl lock-session` locks, suspend locks first, and the
//! session's locked hint follows the lock.
//!
//! logind's `Unlock` signal is deliberately ignored. Any process of the user
//! may ask logind to send it, and the lock comes down only for a password.
//!
//! Locking before suspend holds a delay inhibitor, released once every output
//! has drawn the lock, so the machine never wakes to the desktop.

use std::os::fd::OwnedFd;

use futures_util::StreamExt;
use zbus::{Connection, zvariant::OwnedObjectPath};

use crate::state::{BackendState, State, lock_screen::LockReason};

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;

    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn lock(&self) -> zbus::Result<()>;
}

#[derive(Default)]
pub struct SleepGuard {
    link: Option<(Connection, OwnedObjectPath)>,
    inhibitor: Option<OwnedFd>,
    /// Suspend is waiting for the lock to reach every screen.
    holding: bool,
}

impl State {
    /// Only a session on real hardware is logind's: a nested compositor
    /// shares the host's session and must not lock along with it.
    pub(super) fn start_logind_watch(&mut self) {
        if !matches!(self.backend, BackendState::Kms(_)) {
            return;
        }
        let tasks = self.common.tasks.clone();
        self.common.tasks.spawn(async move {
            if let Err(err) = watch(tasks).await {
                tracing::warn!(%err, "no logind: lock-session and lock-on-suspend are off");
            }
        });
    }

    fn logind_ready(&mut self, connection: Connection, session: OwnedObjectPath) {
        self.lock.sleep.link = Some((connection, session));
        self.refresh_sleep_inhibitor();
        self.announce_lock_state();
    }

    fn on_prepare_for_sleep(&mut self, start: bool) {
        if !start {
            self.refresh_sleep_inhibitor();
            return;
        }
        if !self.lock.scene.current().config.lock_on_suspend {
            self.lock.sleep.inhibitor = None;
            return;
        }
        self.lock_screen(LockReason::Suspend);
        self.lock.sleep.holding = self.lock.sleep.inhibitor.is_some();
        self.queue_redraw();
    }

    /// Lets suspend go ahead once the lock is on every screen.
    pub(super) fn release_sleep_once_locked(&mut self) {
        if !self.lock.sleep.holding {
            return;
        }
        let virtual_outputs = &self.virtual_outputs;
        let outputs: Vec<_> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| monitor.output())
            .filter(|output| !virtual_outputs.contains(output))
            .cloned()
            .collect();
        if self.shell.lock_screen.has_blanked(outputs.iter()) {
            self.lock.sleep.holding = false;
            self.lock.sleep.inhibitor = None;
            tracing::info!("lock drawn; suspend may proceed");
        }
    }

    /// Holds a delay inhibitor exactly while locking on suspend is wanted.
    pub(super) fn refresh_sleep_inhibitor(&mut self) {
        let wanted = self.lock.scene.current().config.lock_on_suspend;
        if !wanted {
            self.lock.sleep.inhibitor = None;
            return;
        }
        if self.lock.sleep.inhibitor.is_some() {
            return;
        }
        let Some((connection, _)) = self.lock.sleep.link.clone() else {
            return;
        };
        self.common.tasks.run(
            async move {
                let manager = ManagerProxy::new(&connection).await?;
                manager
                    .inhibit(
                        "sleep",
                        "CrownOS",
                        "Lock the screen before suspending",
                        "delay",
                    )
                    .await
            },
            |state, inhibitor: zbus::Result<zbus::zvariant::OwnedFd>| match inhibitor {
                Ok(fd) if state.lock.scene.current().config.lock_on_suspend => {
                    state.lock.sleep.inhibitor = Some(fd.into());
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(%err, "no sleep inhibitor: suspend will not wait for the lock")
                }
            },
        );
    }

    pub(super) fn set_logind_locked_hint(&self, locked: bool) {
        let Some((connection, session)) = self.lock.sleep.link.clone() else {
            return;
        };
        self.common.tasks.spawn(async move {
            let hint = async {
                SessionProxy::builder(&connection)
                    .path(session)?
                    .build()
                    .await?
                    .set_locked_hint(locked)
                    .await
            };
            if let Err(err) = hint.await {
                tracing::debug!(%err, "locked hint not set");
            }
        });
    }
}

async fn watch(tasks: crate::utils::runtime::TaskSender) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let manager = ManagerProxy::new(&connection).await?;
    let session = manager.get_session_by_pid(std::process::id()).await?;
    let session_proxy = SessionProxy::builder(&connection)
        .path(session.clone())?
        .build()
        .await?;
    let sleeps = manager
        .receive_prepare_for_sleep()
        .await?
        .map(|signal| Signal::Sleep(signal.args().map(|args| args.start).unwrap_or(false)));
    let locks = session_proxy.receive_lock().await?.map(|_| Signal::Lock);
    tasks.deliver(move |state| state.logind_ready(connection, session));
    let mut signals = futures_util::stream::select(sleeps, locks);
    while let Some(signal) = signals.next().await {
        match signal {
            Signal::Sleep(start) => tasks.deliver(move |state| state.on_prepare_for_sleep(start)),
            Signal::Lock => tasks.deliver(|state| state.lock_screen(LockReason::Requested)),
        }
    }
    Ok(())
}

enum Signal {
    Sleep(bool),
    Lock,
}

//! Turns the accessibility bus on for the whole session before the first app
//! starts, and off again when the engine turned it on and is now switched off.
//!
//! Chromium and Electron read `org.a11y.Status.IsEnabled` only when they
//! start, so an app that starts before the switch never shows its tree. Qt
//! and Firefox also follow a later change; GTK 4 ignores the switch.
//!
//! The visual context engine reads text through this bus, so the switch
//! follows `visual_context.enabled`. The bus launcher keeps the value in
//! gsettings across sessions, so a marker file records that the compositor
//! turned it on. Without the marker the compositor never turns it off: a
//! screen reader may need it.

use std::path::PathBuf;
use std::time::Duration;

use crate::state::{BackendState, State};
use crate::utils::runtime::TaskSender;
use crate::utils::xdg;

/// A missing a11y bus answers at once. The startup apps wait this long for a
/// bus launcher that starts cold; the switch itself goes on in the background.
const STARTUP_WAIT: Duration = Duration::from_millis(500);
const MARKER: &str = "crownos/crownpositor/accessibility-enabled-by-engine";

impl State {
    /// Starts the session apps. When this session owns the host bus, the
    /// accessibility switch follows `visual_context.enabled` first, for at
    /// most [`STARTUP_WAIT`].
    pub fn start_session_apps(&self) {
        if !matches!(self.backend, BackendState::Kms(_)) {
            self.run_startup();
            return;
        }
        let wanted = self.config.current.visual_context.enabled;
        let finished = follow_switch(&self.common.tasks, true, wanted);
        if !wanted {
            self.run_startup();
            return;
        }
        self.common.tasks.run(
            async move { tokio::time::timeout(STARTUP_WAIT, finished).await.is_ok() },
            |state, in_time| {
                if !in_time {
                    tracing::warn!("the accessibility bus is slow; starting the apps without it");
                }
                state.run_startup();
            },
        );
    }
}

/// Sets the switch in the background; the receiver fires when it is done.
pub fn follow_switch(
    tasks: &TaskSender,
    owns_session_bus: bool,
    wanted: bool,
) -> tokio::sync::oneshot::Receiver<()> {
    let (done, finished) = tokio::sync::oneshot::channel();
    if !owns_session_bus {
        let _ = done.send(());
        return finished;
    }
    tasks.spawn(async move {
        match apply_switch(wanted).await {
            Ok(Some(enabled)) => tracing::info!(enabled, "session accessibility switched"),
            Ok(None) => {}
            Err(err) => tracing::warn!(%err, "could not switch session accessibility"),
        }
        let _ = done.send(());
    });
    finished
}

/// The value it set, or `None` when it left the switch alone.
async fn apply_switch(wanted: bool) -> zbus::Result<Option<bool>> {
    let marker = marker_path();
    let marked = marker.as_ref().is_some_and(|marker| marker.exists());
    if !wanted && !marked {
        return Ok(None);
    }
    let bus = zbus::Connection::session().await?;
    if !wanted {
        set_enabled(&bus, false).await?;
        if let Some(marker) = &marker {
            let _ = std::fs::remove_file(marker);
        }
        return Ok(Some(false));
    }
    if is_enabled(&bus).await? {
        return Ok(None);
    }
    set_enabled(&bus, true).await?;
    if let Some(marker) = &marker {
        if let Some(parent) = marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(err) = std::fs::write(marker, b"") {
            tracing::warn!(%err, "cannot record that the engine enabled accessibility");
        }
    }
    Ok(Some(true))
}

fn marker_path() -> Option<PathBuf> {
    xdg::state_home().map(|state| state.join(MARKER))
}

async fn is_enabled(bus: &zbus::Connection) -> zbus::Result<bool> {
    let reply = bus
        .call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.a11y.Status", "IsEnabled"),
        )
        .await?;
    let value: zbus::zvariant::OwnedValue = reply.body().deserialize()?;
    Ok(bool::try_from(value)?)
}

async fn set_enabled(bus: &zbus::Connection, enabled: bool) -> zbus::Result<()> {
    bus.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &(
            "org.a11y.Status",
            "IsEnabled",
            zbus::zvariant::Value::from(enabled),
        ),
    )
    .await?;
    Ok(())
}

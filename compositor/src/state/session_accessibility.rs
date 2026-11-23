//! Turns the accessibility bus on for the whole session before the first app
//! starts.
//!
//! Chromium and Electron read `org.a11y.Status.IsEnabled` only when they
//! start, so an app that starts before the switch never shows its tree. Qt
//! and Firefox also follow a later change; GTK 4 ignores the switch.
//!
//! The visual context engine reads text through this bus, so the switch
//! follows `visual_context.enabled`. The compositor only sets it on: a screen
//! reader may already need it.

use std::time::Duration;

use crate::state::State;

/// A missing a11y bus answers at once; this limit is for a bus launcher that
/// hangs.
const SWITCH_TIMEOUT: Duration = Duration::from_millis(500);

impl State {
    /// Starts the session apps, after the accessibility switch when this
    /// session owns the host bus and the visual context engine is on.
    pub fn start_session_apps(&self, owns_session_bus: bool) {
        if !owns_session_bus || !self.config.current.visual_context.enabled {
            self.run_startup();
            return;
        }
        self.common.tasks.run(
            async { tokio::time::timeout(SWITCH_TIMEOUT, enable_accessibility()).await },
            |state, outcome| {
                match outcome {
                    Ok(Ok(())) => tracing::info!("session accessibility enabled"),
                    Ok(Err(err)) => tracing::warn!(%err, "could not enable session accessibility"),
                    Err(_) => tracing::warn!("the accessibility bus did not answer in time"),
                }
                state.run_startup();
            },
        );
    }
}

async fn enable_accessibility() -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    bus.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &(
            "org.a11y.Status",
            "IsEnabled",
            zbus::zvariant::Value::from(true),
        ),
    )
    .await?;
    Ok(())
}

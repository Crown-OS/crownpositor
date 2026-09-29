//! Hands this session's displays to D-Bus activation and the systemd user
//! manager. Services started on demand — xdg-desktop-portal and its backends
//! above all — inherit their environment from there, not from the compositor,
//! and without this they find no display to connect to.

use std::collections::HashMap;

use crate::state::State;

pub const DESKTOP_NAME: &str = "CrownOS";

impl State {
    pub fn export_session_environment(&self) {
        let desktop =
            std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| DESKTOP_NAME.to_owned());
        let mut variables = vec![
            (
                "WAYLAND_DISPLAY",
                self.common.socket_name.to_string_lossy().into_owned(),
            ),
            ("XDG_CURRENT_DESKTOP", desktop),
            ("XDG_SESSION_TYPE", "wayland".to_owned()),
        ];
        if let Some(display) = self.xwayland.display_name() {
            variables.push(("DISPLAY", display.to_owned()));
        }
        self.common.tasks.spawn(async move {
            if let Err(err) = export(&variables).await {
                tracing::warn!(%err, "could not hand the session environment to D-Bus");
            }
        });
    }
}

async fn export(variables: &[(&str, String)]) -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    let activation: HashMap<&str, &str> = variables
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    bus.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "UpdateActivationEnvironment",
        &(activation,),
    )
    .await?;

    let assignments: Vec<String> = variables
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    // Without a systemd user manager, D-Bus activation alone still reaches
    // the portal.
    if let Err(err) = bus
        .call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "SetEnvironment",
            &(assignments,),
        )
        .await
    {
        tracing::debug!(%err, "no systemd user manager to hand the environment to");
    }
    tracing::info!("session environment exported for activated services");
    Ok(())
}

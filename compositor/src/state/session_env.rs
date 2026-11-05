//! Hands this session's displays to D-Bus activation and the systemd user
//! manager. Services started on demand — xdg-desktop-portal and its backends
//! above all — inherit their environment from there, not from the compositor,
//! and without this they find no display to connect to.

use std::{collections::HashMap, ffi::OsStr};

use crate::state::State;

pub const DESKTOP_NAME: &str = "CrownOS";

const EXPORTED: [&str; 4] = [
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "ELECTRON_OZONE_PLATFORM_HINT",
];

/// Points every child at our socket as a Wayland session.
///
/// Chromium and Electron choose X11 unless `XDG_SESSION_TYPE` is `wayland`
/// (a login shell leaves it at `tty`), and Electron before 38 also needs the
/// ozone hint. Under X11 their own titlebars stop working: xwayland-satellite
/// drops Chromium's `_NET_WM_MOVERESIZE`, which carries button 0, and ignores
/// minimize and maximize requests.
///
/// # Safety
///
/// No other thread may be reading or writing the environment.
pub unsafe fn enter_wayland_session(socket_name: &OsStr) {
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", socket_name);
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        if std::env::var_os("XDG_CURRENT_DESKTOP").is_none() {
            std::env::set_var("XDG_CURRENT_DESKTOP", DESKTOP_NAME);
        }
        if std::env::var_os("ELECTRON_OZONE_PLATFORM_HINT").is_none() {
            std::env::set_var("ELECTRON_OZONE_PLATFORM_HINT", "auto");
        }
    }
}

impl State {
    /// Runs after [`enter_wayland_session`], whose variables it hands on.
    pub fn export_session_environment(&self) {
        let mut variables: Vec<_> = EXPORTED
            .into_iter()
            .filter_map(|name| Some((name, std::env::var(name).ok()?)))
            .collect();
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

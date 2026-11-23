//! Stands in for crownos-secretsd in the nested secret input test: finds the
//! window of an app through `ext_foreign_toplevel_list_v1`, sends one
//! `crownos_secret_input_v1` request and prints its outcome.
//!
//! ```text
//! secret_input_probe <app_id> <require_password_field 0|1> <kind>:<value>[:tab] ...
//! ```
//!
//! `PROBE_CLAIMED_APP_ID` sends another app id than the window has.
//!
//! The values are test values only.

use std::{io::Write as _, os::fd::OwnedFd};

use anyhow::{Context, bail};
use crownos_protocols::secret_input::v1::client::{
    crownos_secret_input_manager_v1::CrownosSecretInputManagerV1,
    crownos_secret_input_request_v1::{self as request, CrownosSecretInputRequestV1, Kind},
};
use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::WlRegistry,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self as handle, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self as list, ExtForeignToplevelListV1},
};

#[derive(Default)]
struct Probe {
    toplevels: Vec<(ExtForeignToplevelHandleV1, String, String)>,
    outcome: Option<WEnum<request::Outcome>>,
}

impl Dispatch<WlRegistry, GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: list::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let list::Event::Toplevel { toplevel } = event {
            state
                .toplevels
                .push((toplevel, String::new(), String::new()));
        }
    }

    wayland_client::event_created_child!(Probe, ExtForeignToplevelListV1, [
        list::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Probe {
    fn event(
        state: &mut Self,
        proxy: &ExtForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.toplevels.iter_mut().find(|entry| entry.0 == *proxy) else {
            return;
        };
        match event {
            handle::Event::Identifier { identifier } => entry.1 = identifier,
            handle::Event::AppId { app_id } => entry.2 = app_id,
            _ => {}
        }
    }
}

impl Dispatch<CrownosSecretInputManagerV1, ()> for Probe {
    fn event(
        _: &mut Self,
        _: &CrownosSecretInputManagerV1,
        _: <CrownosSecretInputManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<CrownosSecretInputRequestV1, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &CrownosSecretInputRequestV1,
        event: request::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let request::Event::Done { outcome } = event {
            state.outcome = Some(outcome);
        }
    }
}

fn sealed_value(text: &str) -> anyhow::Result<OwnedFd> {
    let fd = memfd_create(
        "secret-input-probe",
        MemfdFlags::ALLOW_SEALING | MemfdFlags::CLOEXEC,
    )?;
    let mut file = std::fs::File::from(fd);
    file.write_all(text.as_bytes())?;
    let fd = OwnedFd::from(file);
    fcntl_add_seals(
        &fd,
        SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL,
    )?;
    Ok(fd)
}

fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [app_id, require, values @ ..] = arguments.as_slice() else {
        bail!("usage: secret_input_probe <app_id> <0|1> <kind>:<value>[:tab] ...");
    };
    let connection = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection)?;
    let handle = queue.handle();
    let _list: ExtForeignToplevelListV1 = globals
        .bind(&handle, 1..=1, ())
        .context("no foreign toplevel list")?;
    let manager: CrownosSecretInputManagerV1 = globals
        .bind(&handle, 1..=1, ())
        .context("the secret input global is not offered")?;
    let mut probe = Probe::default();
    queue.roundtrip(&mut probe)?;
    queue.roundtrip(&mut probe)?;
    let identifier = probe
        .toplevels
        .iter()
        .find(|entry| entry.2 == *app_id)
        .map(|entry| entry.1.clone())
        .with_context(|| {
            let known: Vec<&str> = probe
                .toplevels
                .iter()
                .map(|entry| entry.2.as_str())
                .collect();
            format!("no window of that app among {known:?}")
        })?;
    let claimed_app_id = std::env::var("PROBE_CLAIMED_APP_ID").unwrap_or_else(|_| app_id.clone());
    let request = manager.create_request(
        identifier,
        claimed_app_id,
        require.parse()?,
        "probe agent".into(),
        "probe secret".into(),
        "https://login.example.com".into(),
        &handle,
        (),
    );
    for value in values {
        let mut parts = value.splitn(3, ':');
        let kind = match parts.next() {
            Some("username") => Kind::Username,
            _ => Kind::Password,
        };
        let text = parts.next().unwrap_or_default();
        let tab_after = u32::from(parts.next() == Some("tab"));
        request.add_value(
            std::os::fd::AsFd::as_fd(&sealed_value(text)?),
            kind,
            tab_after,
        );
    }
    request.commit();
    while probe.outcome.is_none() {
        queue.blocking_dispatch(&mut probe)?;
    }
    println!("outcome {:?}", probe.outcome);
    Ok(())
}

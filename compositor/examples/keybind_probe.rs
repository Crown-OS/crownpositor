//! Registers application keybinds through `crownos_keybind_v1` and prints
//! every event, so the assignment rules and the press, hold and release
//! events can be checked against a running compositor.
//!
//! ```text
//! cargo run -p compositor --example keybind_probe -- <app_id> <name>=<keys>...
//! cargo run -p compositor --example keybind_probe -- probe record=Super+Alt+R talk=Super+Alt+T
//! ```

use std::time::Duration;

use anyhow::{Context, bail};
use crownos_protocols::keybind::v1::client::{
    crownos_keybind_manager_v1::CrownosKeybindManagerV1,
    crownos_keybind_v1::{CrownosKeybindV1, Event},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};

/// A press shorter than this is a click; a longer one is a hold.
const HOLD_THRESHOLD: Duration = Duration::from_millis(400);

#[derive(Default)]
struct Probe {
    pressed_at: Vec<Option<u32>>,
}

fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let app_id = arguments
        .next()
        .context("usage: keybind_probe <app_id> <name>=<keys>...")?;
    let actions: Vec<(String, String)> = arguments
        .map(|argument| match argument.split_once('=') {
            Some((name, keys)) => Ok((name.to_owned(), keys.to_owned())),
            None => bail!("`{argument}` is not <name>=<keys>"),
        })
        .collect::<anyhow::Result<_>>()?;

    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection)?;
    let queue_handle = queue.handle();
    let manager: CrownosKeybindManagerV1 = globals
        .bind(&queue_handle, 1..=1, ())
        .context("the compositor does not implement crownos_keybind_v1")?;

    let mut probe = Probe {
        pressed_at: vec![None; actions.len()],
    };
    let _keybinds: Vec<CrownosKeybindV1> = actions
        .iter()
        .enumerate()
        .map(|(index, (name, keys))| {
            manager.register(
                app_id.clone(),
                name.clone(),
                format!("probe action {name}"),
                keys.clone(),
                &queue_handle,
                (index, name.clone()),
            )
        })
        .collect();

    loop {
        queue.blocking_dispatch(&mut probe)?;
    }
}

impl Dispatch<CrownosKeybindV1, (usize, String)> for Probe {
    fn event(
        probe: &mut Self,
        _keybind: &CrownosKeybindV1,
        event: Event,
        (index, name): &(usize, String),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            Event::Assigned { keys } => println!("{name}: assigned {keys}"),
            Event::Pressed { time } => {
                probe.pressed_at[*index] = Some(time);
                println!("{name}: pressed at {time}");
            }
            Event::Released { time } => {
                let held_for = probe.pressed_at[*index]
                    .take()
                    .map(|pressed| Duration::from_millis(time.wrapping_sub(pressed).into()));
                let kind = match held_for {
                    Some(duration) if duration >= HOLD_THRESHOLD => "hold",
                    Some(_) => "click",
                    None => "release without a press",
                };
                println!("{name}: released at {time} after {held_for:?}: {kind}");
            }
            Event::Cancelled => {
                probe.pressed_at[*index] = None;
                println!("{name}: cancelled");
            }
            _ => {}
        }
    }
}

impl Dispatch<CrownosKeybindManagerV1, ()> for Probe {
    fn event(
        _: &mut Self,
        _: &CrownosKeybindManagerV1,
        _: <CrownosKeybindManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

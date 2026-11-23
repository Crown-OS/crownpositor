//! Presses keys through `zwp_virtual_keyboard_v1`, with the keymap the seat
//! itself sends, for the nested consent test:
//!
//! ```text
//! cargo run -p compositor --example virtual_keyboard_probe -- 1 58
//! ```
//!
//! The arguments are evdev key codes; 1 is Escape, or Caps Lock when the
//! seat swaps the two, and 58 the other way round.

use std::os::fd::OwnedFd;

use anyhow::{Context, bail};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_keyboard::{self, KeyState, KeymapFormat, WlKeyboard},
        wl_registry::WlRegistry,
        wl_seat::WlSeat,
    },
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

#[derive(Default)]
struct Probe {
    keymap: Option<(OwnedFd, u32)>,
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

impl Dispatch<WlSeat, ()> for Probe {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wayland_client::protocol::wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlKeyboard, ()> for Probe {
    fn event(
        probe: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap {
            format: WEnum::Value(KeymapFormat::XkbV1),
            fd,
            size,
        } = event
        {
            probe.keymap = Some((fd, size));
        }
    }
}

impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for Probe {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardManagerV1,
        _: <ZwpVirtualKeyboardManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, ()> for Probe {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardV1,
        _: <ZwpVirtualKeyboardV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn main() -> anyhow::Result<()> {
    let keys = std::env::args()
        .skip(1)
        .map(|key| key.parse::<u32>())
        .collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() {
        bail!("usage: virtual_keyboard_probe KEY...");
    }
    let connection = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection)?;
    let handle = queue.handle();
    let seat: WlSeat = globals.bind(&handle, 1..=7, ())?;
    let _keyboard = seat.get_keyboard(&handle, ());
    let manager: ZwpVirtualKeyboardManagerV1 = globals
        .bind(&handle, 1..=1, ())
        .context("no virtual keyboard manager")?;
    let mut probe = Probe::default();
    queue.roundtrip(&mut probe)?;
    let Some((keymap, size)) = probe.keymap.take() else {
        bail!("the seat sent no keymap");
    };
    let keyboard = manager.create_virtual_keyboard(&seat, &handle, ());
    keyboard.keymap(
        KeymapFormat::XkbV1 as u32,
        std::os::fd::AsFd::as_fd(&keymap),
        size,
    );
    for (time, key) in (0u32..).step_by(2).zip(&keys) {
        keyboard.key(time, *key, KeyState::Pressed as u32);
        keyboard.key(time + 1, *key, KeyState::Released as u32);
    }
    queue.roundtrip(&mut probe)?;
    println!("sent keys {keys:?}");
    Ok(())
}

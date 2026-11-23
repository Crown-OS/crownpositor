//! Takes the keyboard focus the way a consent dialog or a screen locker does, then asks for a
//! new `wl_keyboard` while it has the focus, for the nested keyboard focus test:
//!
//! ```text
//! keyboard_focus_probe consent   # a consent layer with exclusive keyboard focus
//! keyboard_focus_probe lock      # an ext-session-lock surface
//! ```
//!
//! Fails when the surface never gets the keyboard focus. Prints `survived` when the compositor
//! still answers after the new keyboard.

use std::{fs::File, io::Write as _, os::fd::AsFd, time::Duration};

use anyhow::{Context, bail};
use rustix::fs::MemfdFlags;
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_keyboard::WlKeyboard,
        wl_output::WlOutput, wl_registry::WlRegistry, wl_seat::WlSeat, wl_shm, wl_shm::WlShm,
        wl_shm_pool::WlShmPool, wl_surface::WlSurface,
    },
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

const NAMESPACE: &str = "crownos-portal-dialog";
const SIZE: u32 = 64;
/// Long enough for the compositor to move the keyboard focus to the new surface.
const FOCUS_SETTLES: Duration = Duration::from_millis(800);

#[derive(Default)]
struct Probe {
    configure: Option<(u32, u32, u32)>,
    locked: bool,
    keyboard_entered: bool,
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

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            state.configure = Some((serial, width, height));
        }
    }
}

impl Dispatch<ExtSessionLockV1, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            state.locked = true;
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            state.configure = Some((serial, width, height));
        }
    }
}

impl Dispatch<WlKeyboard, ()> for Probe {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wayland_client::protocol::wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_keyboard::Event::Enter { .. } = event {
            state.keyboard_entered = true;
        }
    }
}

delegate_noop!(Probe: ignore WlCompositor);
delegate_noop!(Probe: ignore WlSurface);
delegate_noop!(Probe: ignore WlShm);
delegate_noop!(Probe: ignore WlShmPool);
delegate_noop!(Probe: ignore WlBuffer);
delegate_noop!(Probe: ignore WlSeat);
delegate_noop!(Probe: ignore WlOutput);
delegate_noop!(Probe: ignore ZwlrLayerShellV1);
delegate_noop!(Probe: ignore ExtSessionLockManagerV1);

fn main() -> anyhow::Result<()> {
    let mode = std::env::args().nth(1).context("say consent or lock")?;
    let connection = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection)?;
    let handle = queue.handle();
    let compositor: WlCompositor = globals.bind(&handle, 4..=6, ())?;
    let shm: WlShm = globals.bind(&handle, 1..=1, ())?;
    let seat: WlSeat = globals.bind(&handle, 1..=9, ())?;
    let _first_keyboard = seat.get_keyboard(&handle, ());
    let surface = compositor.create_surface(&handle, ());
    let mut probe = Probe::default();

    let lock = match mode.as_str() {
        "consent" => {
            let layers: ZwlrLayerShellV1 = globals.bind(&handle, 1..=4, ())?;
            let layer = layers.get_layer_surface(
                &surface,
                None,
                Layer::Overlay,
                NAMESPACE.to_owned(),
                &handle,
                (),
            );
            layer.set_size(SIZE, SIZE);
            layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            surface.commit();
            while probe.configure.is_none() {
                queue.blocking_dispatch(&mut probe)?;
            }
            if let Some((serial, _, _)) = probe.configure {
                layer.ack_configure(serial);
            }
            None
        }
        "lock" => {
            let manager: ExtSessionLockManagerV1 = globals.bind(&handle, 1..=1, ())?;
            let output: WlOutput = globals.bind(&handle, 1..=4, ())?;
            let lock = manager.lock(&handle, ());
            let lock_surface = lock.get_lock_surface(&surface, &output, &handle, ());
            while probe.configure.is_none() {
                queue.blocking_dispatch(&mut probe)?;
            }
            if let Some((serial, _, _)) = probe.configure {
                lock_surface.ack_configure(serial);
            }
            Some(lock)
        }
        other => bail!("unknown mode {other}"),
    };
    let (width, height) = probe.configure.map_or((SIZE, SIZE), |(_, width, height)| {
        (width.max(1), height.max(1))
    });
    let stride = width * 4;
    let length = usize::try_from(stride * height)?;
    let mut memfd = File::from(rustix::fs::memfd_create(
        "keyboard-focus-probe",
        MemfdFlags::CLOEXEC,
    )?);
    memfd.write_all(&vec![0x80; length])?;
    let pool = shm.create_pool(memfd.as_fd(), i32::try_from(length)?, &handle, ());
    let buffer = pool.create_buffer(
        0,
        i32::try_from(width)?,
        i32::try_from(height)?,
        i32::try_from(stride)?,
        wl_shm::Format::Argb8888,
        &handle,
        (),
    );
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    queue.roundtrip(&mut probe)?;
    std::thread::sleep(FOCUS_SETTLES);
    queue.roundtrip(&mut probe)?;
    if lock.is_some() && !probe.locked {
        bail!("the session did not lock");
    }
    if !probe.keyboard_entered {
        bail!("the {mode} surface never got the keyboard focus");
    }

    let _second_keyboard = seat.get_keyboard(&handle, ());
    queue.roundtrip(&mut probe)?;
    std::thread::sleep(Duration::from_millis(200));
    queue.roundtrip(&mut probe)?;
    if let Some(lock) = lock {
        lock.unlock_and_destroy();
        queue.roundtrip(&mut probe)?;
    }
    println!("survived");
    Ok(())
}

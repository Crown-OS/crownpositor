//! `spawn`: shm toplevels that stay open until the probe is stopped. The top
//! half of each is lighter than the bottom, so a flipped output shows.

use std::{
    fs::File,
    io::Write,
    os::{fd::AsFd, unix::process::CommandExt},
    process::Command,
};

use anyhow::Context;
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_compositor::WlCompositor,
        wl_registry,
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};

const WORKSPACE_VARIABLE: &str = "WORKSPACE";
const DEFAULT_SIZE: (i32, i32) = (480, 320);
const BYTES_PER_PIXEL: i32 = 4;

struct Window {
    surface: WlSurface,
    size: (i32, i32),
    shade: u8,
}

struct Windows {
    shm: WlShm,
    windows: Vec<Window>,
}

pub fn run(count: usize, app_id: &str, workspace: Option<u32>) -> anyhow::Result<()> {
    // The compositor reads WORKSPACE from the environment the process
    // started with, so a changed variable needs a fresh exec.
    if let Some(workspace) = workspace.map(|workspace| workspace.to_string())
        && std::env::var(WORKSPACE_VARIABLE).ok().as_deref() != Some(workspace.as_str())
    {
        let error = Command::new(std::env::current_exe()?)
            .args(std::env::args_os().skip(1))
            .env(WORKSPACE_VARIABLE, workspace)
            .exec();
        return Err(error).context("could not restart with WORKSPACE set");
    }

    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Windows>(&connection)?;
    let handle = queue.handle();
    let compositor: WlCompositor = globals.bind(&handle, 4..=6, ())?;
    let wm_base: XdgWmBase = globals.bind(&handle, 1..=6, ())?;
    let mut state = Windows {
        shm: globals.bind(&handle, 1..=1, ())?,
        windows: Vec::new(),
    };

    for index in 0..count {
        let surface = compositor.create_surface(&handle, ());
        let toplevel = wm_base
            .get_xdg_surface(&surface, &handle, index)
            .get_toplevel(&handle, index);
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(format!("{app_id} {}", index + 1));
        surface.commit();
        state.windows.push(Window {
            surface,
            size: DEFAULT_SIZE,
            shade: 0x60 + (index as u8 % 4) * 0x28,
        });
    }
    println!("spawned {count} windows with app id {app_id}; Ctrl-C closes them");
    loop {
        queue.blocking_dispatch(&mut state)?;
    }
}

impl Windows {
    fn draw(&self, handle: &QueueHandle<Self>, index: usize) -> anyhow::Result<()> {
        let window = &self.windows[index];
        let (width, height) = window.size;
        let stride = width * BYTES_PER_PIXEL;
        let mut pixels = Vec::with_capacity((stride * height) as usize);
        for row in 0..height {
            let shade = if row < height / 2 {
                window.shade.saturating_add(0x50)
            } else {
                window.shade
            };
            for _ in 0..width {
                pixels.extend_from_slice(&[shade / 2, shade / 3, shade, 0xff]);
            }
        }
        let mut file = shm_file()?;
        file.write_all(&pixels)?;
        let pool = self
            .shm
            .create_pool(file.as_fd(), stride * height, handle, ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            stride,
            wl_shm::Format::Argb8888,
            handle,
            (),
        );
        pool.destroy();
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage_buffer(0, 0, width, height);
        window.surface.commit();
        Ok(())
    }
}

/// An unlinked file in the runtime directory to share pixels through.
fn shm_file() -> anyhow::Result<File> {
    let directory = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    let path = std::path::Path::new(&directory)
        .join(format!("multi-monitor-probe-{}", std::process::id()));
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    std::fs::remove_file(&path)?;
    Ok(file)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Windows {
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

impl Dispatch<XdgWmBase, ()> for Windows {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, usize> for Windows {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        index: &usize,
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if let Err(error) = state.draw(handle, *index) {
                eprintln!("window {index}: {error:#}");
            }
        }
    }
}

impl Dispatch<XdgToplevel, usize> for Windows {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event
            && width > 0
            && height > 0
            && let Some(window) = state.windows.get_mut(*index)
        {
            window.size = (width, height);
        }
    }
}

delegate_noop!(Windows: ignore WlCompositor);
delegate_noop!(Windows: ignore WlSurface);
delegate_noop!(Windows: ignore WlShm);
delegate_noop!(Windows: ignore WlShmPool);
delegate_noop!(Windows: ignore WlBuffer);

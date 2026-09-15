//! A window made of nothing but `crownos_background_effects`.
//!
//! Commits a fully transparent buffer and asks the compositor for the material
//! instead: a shape of two pills and two circles, blurred and tinted, with a
//! refractive rim along every edge and a shadow underneath. Whatever you see is
//! the compositor's, which is what makes this the check that the protocol and
//! the renderer agree.
//!
//! ```text
//! cargo run -p compositor --example liquid_glass
//! ```

use std::{fs::File, os::fd::AsFd};

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_compositor::WlCompositor,
        wl_registry::WlRegistry,
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

use crownos_protocols::background_effects::v1::client::{
    crownos_background_effect_manager_v1::{self, CrownosBackgroundEffectManagerV1},
    crownos_background_effect_surface_v1::CrownosBackgroundEffectSurfaceV1,
    crownos_shape_manager_v1::CrownosShapeManagerV1,
    crownos_shape_v1::CrownosShapeV1,
};

const WIDTH: i32 = 900;
const HEIGHT: i32 = 320;
/// Corner radius of the window itself, which is also what the shadow and the
/// whole-surface effects are cut from.
const RADIUS: u32 = 32;
/// Width of the refractive rim, in surface-local pixels.
const RIM: u32 = 3;
/// 130%: the vibrancy that keeps a wallpaper's colour alive through the blur.
const SATURATION: f64 = 1.3;

#[derive(Default)]
struct App {
    configured: bool,
    closed: bool,
}

fn main() {
    let connection = Connection::connect_to_env().expect("no Wayland compositor to talk to");
    let (globals, mut queue) =
        registry_queue_init::<App>(&connection).expect("the registry is unreadable");
    let qh = queue.handle();

    let compositor: WlCompositor = globals.bind(&qh, 1..=6, ()).expect("no wl_compositor");
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).expect("no wl_shm");
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("no xdg_wm_base");
    let shapes: CrownosShapeManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .expect("the compositor does not implement crownos_shape_manager_v1");
    let effects: CrownosBackgroundEffectManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .expect("the compositor does not implement crownos_background_effect_manager_v1");

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Liquid Glass".to_owned());
    toplevel.set_app_id("crownos.LiquidGlass".to_owned());
    surface.commit();

    let mut app = App::default();
    while !app.configured {
        queue.blocking_dispatch(&mut app).expect("dispatch failed");
    }

    let effect = effects.get_effect_surface(&surface, &qh, ());
    // The window's own outline: what the shadow is cast from, and what the
    // whole-surface effects would be clipped to.
    effect.set_corner_radius(RADIUS);
    effect.set_border(RIM);

    // A control-centre row: two tall pills flanked by round buttons. One shape,
    // four primitives, one signed distance field per piece — which is the whole
    // argument for parametric geometry over a `wl_region`.
    let shape = shapes.create_shape(&qh, ());
    shape.add_rounded_rect(40, 40, 140, 240, 70);
    shape.add_rounded_rect(200, 40, 140, 240, 70);
    shape.add_circle(440, 110, 60);
    shape.add_circle(440, 250, 60);
    // A shape is copied when it is used, so this is free to go right away.
    effect.set_blur(Some(&shape), 48, 0x20_ff_ff_ff, SATURATION);
    effect.set_shadow(None, 40, 0, 12, 0x60_00_00_00);
    shape.destroy();

    let buffer = transparent_buffer(&shm, &qh);
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, WIDTH, HEIGHT);
    surface.commit();

    while !app.closed {
        queue.blocking_dispatch(&mut app).expect("dispatch failed");
    }
}

/// A fully transparent ARGB8888 buffer.
///
/// No pixels are ever written: a freshly sized file reads as zeroes, and zero in
/// premultiplied ARGB8888 *is* transparent. So the window contributes nothing at
/// all, and everything on screen is the compositor's material.
fn transparent_buffer(shm: &WlShm, qh: &QueueHandle<App>) -> WlBuffer {
    let stride = WIDTH * 4;
    let size = stride * HEIGHT;

    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(scratch_path())
        .expect("cannot create the shm file");
    std::fs::remove_file(scratch_path()).expect("cannot unlink the shm file");
    file.set_len(size as u64).expect("cannot size the shm file");

    let pool = shm.create_pool(file.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(0, WIDTH, HEIGHT, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

fn scratch_path() -> std::path::PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_owned());
    std::path::Path::new(&dir).join(format!("liquid-glass-{}", std::process::id()))
}

impl Dispatch<XdgWmBase, ()> for App {
    fn event(
        _app: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for App {
    fn event(
        app: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            app.configured = true;
        }
    }
}

impl Dispatch<XdgToplevel, ()> for App {
    fn event(
        app: &mut Self,
        _toplevel: &XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            app.closed = true;
        }
    }
}

impl Dispatch<CrownosBackgroundEffectManagerV1, ()> for App {
    fn event(
        _app: &mut Self,
        _manager: &CrownosBackgroundEffectManagerV1,
        event: crownos_background_effect_manager_v1::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // What the compositor will actually draw. A client that needs its text
        // legible watches this and paints its own background when blur goes
        // away; this one just says what it was told.
        if let crownos_background_effect_manager_v1::Event::Capabilities { capabilities } = event {
            println!("capabilities: {capabilities:?}");
        }
    }
}

/// Everything that never sends an event.
macro_rules! silent {
    ($($interface:ty),* $(,)?) => {
        $(impl Dispatch<$interface, ()> for App {
            fn event(
                _app: &mut Self,
                _proxy: &$interface,
                _event: <$interface as wayland_client::Proxy>::Event,
                _data: &(),
                _connection: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
            }
        })*
    };
}

silent!(
    WlCompositor,
    WlSurface,
    WlShm,
    WlShmPool,
    WlBuffer,
    CrownosShapeManagerV1,
    CrownosShapeV1,
    CrownosBackgroundEffectSurfaceV1,
);

impl Dispatch<WlRegistry, GlobalListContents> for App {
    fn event(
        _app: &mut Self,
        _registry: &WlRegistry,
        _event: <WlRegistry as wayland_client::Proxy>::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

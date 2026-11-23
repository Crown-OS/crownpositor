//! Stands in for `crownos-portal-dialog` in nested consent tests, started
//! through `CROWN_CONSENT_DIALOG`. It writes its arguments to
//! `$CONSENT_PROBE_LOG`, then does what `$CONSENT_PROBE_SCRIPT` says:
//!
//! ```text
//! <choice id or "decline"> <"layer" or "no-layer"> <milliseconds to wait>
//! ```
//!
//! With `layer` it maps an overlay layer in the dialog's namespace first.

use std::{io::Write as _, time::Duration};

use anyhow::{Context, bail};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_surface::WlSurface},
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

const NAMESPACE: &str = "crownos-portal-dialog";

struct Probe;

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

wayland_client::delegate_noop!(Probe: ignore WlCompositor);
wayland_client::delegate_noop!(Probe: ignore WlSurface);
wayland_client::delegate_noop!(Probe: ignore ZwlrLayerShellV1);

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Probe {
    fn event(
        _: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            layer.ack_configure(serial);
        }
    }
}

fn show_layer() -> anyhow::Result<(Connection, ZwlrLayerSurfaceV1)> {
    let connection = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection)?;
    let handle = queue.handle();
    let compositor: WlCompositor = globals.bind(&handle, 1..=6, ())?;
    let shell: ZwlrLayerShellV1 = globals.bind(&handle, 1..=4, ())?;
    let surface = compositor.create_surface(&handle, ());
    let layer = shell.get_layer_surface(
        &surface,
        None,
        Layer::Overlay,
        NAMESPACE.into(),
        &handle,
        (),
    );
    layer.set_anchor(Anchor::all());
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    surface.commit();
    queue.roundtrip(&mut Probe)?;
    Ok((connection, layer))
}

fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(log) = std::env::var_os("CONSENT_PROBE_LOG") {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)?;
        writeln!(file, "{}", arguments.join(" | "))?;
    }
    let script = std::fs::read_to_string(
        std::env::var_os("CONSENT_PROBE_SCRIPT").context("CONSENT_PROBE_SCRIPT is not set")?,
    )?;
    let [answer, layer, wait] = script.split_whitespace().collect::<Vec<_>>()[..] else {
        bail!("the script needs: answer layer milliseconds");
    };
    let _shown = (layer == "layer").then(show_layer).transpose()?;
    std::thread::sleep(Duration::from_millis(wait.parse()?));
    if answer == "decline" {
        std::process::exit(1);
    }
    println!("{answer}");
    Ok(())
}

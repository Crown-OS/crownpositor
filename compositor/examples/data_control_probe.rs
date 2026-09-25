//! Checks the two ext-data-control guarantees that live in the compositor
//! rather than in the protocol XML:
//!
//! - a client connected through a `wp_security_context_v1` listener, which is
//!   how Flatpak and other sandboxes connect their apps, never sees the
//!   privileged globals;
//! - a source handed to `set_selection` twice is a `used_source` protocol
//!   error, not a silent re-offer.
//!
//! ```text
//! WAYLAND_DISPLAY=wayland-1 cargo run -p compositor --example data_control_probe
//! ```

use std::os::{
    fd::AsFd,
    unix::net::{UnixListener, UnixStream},
};

use anyhow::{Context, Result, bail, ensure};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop, event_created_child,
    globals::{GlobalList, GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_seat::WlSeat},
};
use wayland_protocols::{
    ext::data_control::v1::client::{
        ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
        ext_data_control_manager_v1::ExtDataControlManagerV1,
        ext_data_control_offer_v1::ExtDataControlOfferV1,
        ext_data_control_source_v1::ExtDataControlSourceV1,
    },
    wp::security_context::v1::client::{
        wp_security_context_manager_v1::WpSecurityContextManagerV1,
        wp_security_context_v1::WpSecurityContextV1,
    },
};

const PRIVILEGED_GLOBALS: [&str; 6] = [
    "ext_data_control_manager_v1",
    "zwlr_data_control_manager_v1",
    "zwlr_gamma_control_manager_v1",
    "zwlr_output_manager_v1",
    "zwlr_output_power_manager_v1",
    "ext_session_lock_manager_v1",
];

struct Probe;

fn main() -> Result<()> {
    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection)?;
    ensure!(
        advertises(&globals, PRIVILEGED_GLOBALS[0]),
        "a host client should see ext_data_control_manager_v1"
    );

    let hidden = sandboxed_globals(&globals, &mut queue)?;
    println!("sandboxed client sees none of: {}", hidden.join(", "));

    source_reuse_is_rejected()?;
    println!("reusing a source is a used_source protocol error");
    Ok(())
}

fn advertises(globals: &GlobalList, interface: &str) -> bool {
    globals
        .contents()
        .with_list(|list| list.iter().any(|global| global.interface == interface))
}

/// Registers a security context the way a sandbox engine would, connects
/// through it, and returns the privileged globals the sandboxed client was
/// denied. Fails if any of them leaked through.
fn sandboxed_globals(
    globals: &GlobalList,
    queue: &mut EventQueue<Probe>,
) -> Result<Vec<&'static str>> {
    let qh = queue.handle();
    let manager: WpSecurityContextManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .context("the compositor does not implement wp_security_context_v1")?;

    let socket_path =
        std::env::temp_dir().join(format!("data-control-probe-{}", std::process::id()));
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path)?;
    let (close_reader, _close_writer) = std::io::pipe()?;

    let context = manager.create_listener(listener.as_fd(), close_reader.as_fd(), &qh, ());
    context.set_sandbox_engine("org.crownos.data-control-probe".into());
    context.set_app_id("data-control-probe".into());
    context.commit();
    queue.roundtrip(&mut Probe)?;

    let sandboxed = Connection::from_socket(UnixStream::connect(&socket_path)?)?;
    let (sandboxed_globals, _queue) = registry_queue_init::<Probe>(&sandboxed)?;
    let _ = std::fs::remove_file(&socket_path);
    ensure!(
        advertises(&sandboxed_globals, "wl_compositor"),
        "the sandboxed client got no globals at all"
    );

    let leaked: Vec<_> = PRIVILEGED_GLOBALS
        .into_iter()
        .filter(|interface| advertises(&sandboxed_globals, interface))
        .collect();
    if !leaked.is_empty() {
        bail!("a sandboxed client can see {}", leaked.join(", "));
    }
    Ok(PRIVILEGED_GLOBALS.to_vec())
}

fn source_reuse_is_rejected() -> Result<()> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Probe>(&connection)?;
    let qh = queue.handle();
    let manager: ExtDataControlManagerV1 = globals.bind(&qh, 1..=1, ())?;
    let seat: WlSeat = globals.bind(&qh, 1..=1, ())?;

    let device = manager.get_data_device(&seat, &qh, ());
    let source = manager.create_data_source(&qh, ());
    source.offer("text/plain;charset=utf-8".into());
    device.set_selection(Some(&source));
    device.set_selection(Some(&source));

    ensure!(
        queue.roundtrip(&mut Probe).is_err(),
        "the compositor accepted the same source twice"
    );
    let error = connection
        .protocol_error()
        .context("the connection died without a protocol error")?;
    ensure!(
        error.object_interface == ExtDataControlDeviceV1::interface().name
            && error.code == ext_data_control_device_v1::Error::UsedSource as u32,
        "expected used_source, got {error}"
    );
    Ok(())
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

impl Dispatch<ExtDataControlDeviceV1, ()> for Probe {
    fn event(
        _: &mut Self,
        _: &ExtDataControlDeviceV1,
        _: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(Probe, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

delegate_noop!(Probe: ignore WlSeat);
delegate_noop!(Probe: ignore ExtDataControlOfferV1);
delegate_noop!(Probe: ignore ExtDataControlSourceV1);
delegate_noop!(Probe: ExtDataControlManagerV1);
delegate_noop!(Probe: WpSecurityContextManagerV1);
delegate_noop!(Probe: WpSecurityContextV1);

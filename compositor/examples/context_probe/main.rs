//! Binds `crownos_context_manager_v1` at version 2 and prints one line per
//! event until it is killed, for the nested context test:
//!
//! ```text
//! cargo run -p compositor --example context_probe
//! ```

use std::io::Write;

use anyhow::Context;
use crownos_protocols::context::v1::client::{
    crownos_context_manager_v1::{self as manager, CrownosContextManagerV1},
    crownos_context_window_v1::{self as window, CrownosContextWindowV1},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::WlRegistry,
};

const VERSION: u32 = 2;

struct Printer;

fn print_line(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

impl Dispatch<WlRegistry, GlobalListContents> for Printer {
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

impl Dispatch<CrownosContextManagerV1, ()> for Printer {
    fn event(
        _: &mut Self,
        _: &CrownosContextManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::Window { identifier, .. } => {
                print_line(&format!("window {identifier}"))
            }
            manager::Event::Scope { .. } => print_line("scope"),
            manager::Event::Revoked { .. } => print_line("revoked"),
            manager::Event::Done => print_line("done"),
            _ => {}
        }
    }

    wayland_client::event_created_child!(Printer, CrownosContextManagerV1, [
        manager::EVT_WINDOW_OPCODE => (CrownosContextWindowV1, ()),
    ]);
}

impl Dispatch<CrownosContextWindowV1, ()> for Printer {
    fn event(
        _: &mut Self,
        _: &CrownosContextWindowV1,
        event: window::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            window::Event::AppId { app_id } => print_line(&format!("app_id {app_id}")),
            window::Event::Title { .. } => print_line("title"),
            window::Event::Closed => print_line("closed"),
            window::Event::Sensitivity { sensitivity } => print_line(&format!(
                "sensitivity {:?} at {}",
                sensitivity,
                monotonic_milliseconds()
            )),
            _ => {}
        }
    }
}

/// `CLOCK_MONOTONIC` in milliseconds, comparable with other processes' clocks.
fn monotonic_milliseconds() -> i64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    now.tv_sec * 1000 + now.tv_nsec / 1_000_000
}

fn main() -> anyhow::Result<()> {
    let connection = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Printer>(&connection)?;
    let _manager: CrownosContextManagerV1 = globals
        .bind(&queue.handle(), VERSION..=VERSION, ())
        .context("the context global is not offered")?;
    print_line("bound");
    loop {
        queue.blocking_dispatch(&mut Printer)?;
    }
}

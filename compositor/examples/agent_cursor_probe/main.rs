//! Creates an agent cursor, then one more for each `again` line on standard input, and prints
//! what the compositor answers for each, for the nested agent cursor test:
//!
//! ```text
//! created <seat>
//! revoked <reason>
//! ```

use std::io::{BufRead as _, Write as _};

use anyhow::Context;
use crownos_protocols::agent_cursor::v1::client::{
    crownos_agent_cursor_manager_v1::CrownosAgentCursorManagerV1,
    crownos_agent_cursor_v1::{self as cursor, CrownosAgentCursorV1},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry::WlRegistry,
};

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

impl Dispatch<CrownosAgentCursorV1, ()> for Printer {
    fn event(
        _: &mut Self,
        _: &CrownosAgentCursorV1,
        event: cursor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            cursor::Event::Created { seat } => print_line(&format!("created {seat}")),
            cursor::Event::Revoked { reason } => {
                let reason = match reason {
                    WEnum::Value(reason) => format!("{reason:?}").to_lowercase(),
                    WEnum::Unknown(value) => value.to_string(),
                };
                print_line(&format!("revoked {reason}"));
            }
            _ => {}
        }
    }
}

delegate_noop!(Printer: ignore CrownosAgentCursorManagerV1);

fn main() -> anyhow::Result<()> {
    let connection = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Printer>(&connection)?;
    let manager: CrownosAgentCursorManagerV1 = globals
        .bind(&queue.handle(), 1..=1, ())
        .context("the agent cursor global is not offered")?;
    let mut cursors =
        vec![manager.create_cursor("probe".to_owned(), 0x00_80_ff, &queue.handle(), ())];
    queue.roundtrip(&mut Printer)?;
    let (lines, input) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                return;
            }
        }
    });
    loop {
        queue.roundtrip(&mut Printer)?;
        match input.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(line) if line == "again" => {
                cursors.push(manager.create_cursor(
                    "probe".to_owned(),
                    0x00_80_ff,
                    &queue.handle(),
                    (),
                ));
            }
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

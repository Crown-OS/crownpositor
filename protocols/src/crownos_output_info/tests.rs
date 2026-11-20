use std::{os::unix::net::UnixStream, sync::Arc};

use wayland_client::{
    Connection, Dispatch as ClientDispatch, EventQueue, QueueHandle,
    protocol::wl_registry::{self, WlRegistry},
};
use wayland_server::{Display, delegate_dispatch, delegate_global_dispatch};

use super::*;

struct Server {
    state: OutputInfoState,
}

impl OutputInfoHandler for Server {
    fn output_info_state(&mut self) -> &mut OutputInfoState {
        &mut self.state
    }

    fn output_info(&mut self, _output: &Output) -> Option<OutputInfo> {
        None
    }
}

delegate_global_dispatch!(Server: [CrownosOutputInfoManagerV1: OutputInfoGlobalData] => OutputInfoState);
delegate_dispatch!(Server: [CrownosOutputInfoManagerV1: ()] => OutputInfoState);
delegate_dispatch!(Server: [CrownosOutputInfoV1: ()] => OutputInfoState);

#[derive(Default)]
struct Offered {
    globals: Vec<(String, u32)>,
}

impl ClientDispatch<WlRegistry, ()> for Offered {
    fn event(
        offered: &mut Self,
        _: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            interface, version, ..
        } = event
        {
            offered.globals.push((interface, version));
        }
    }
}

fn offered_globals(privileged: bool) -> Result<Vec<(String, u32)>, Box<dyn std::error::Error>> {
    let mut display = Display::<Server>::new()?;
    let mut server = Server {
        state: OutputInfoState::new::<Server, _>(&display.handle(), move |_| privileged),
    };
    let (server_end, client_end) = UnixStream::pair()?;
    display.handle().insert_client(server_end, Arc::new(()))?;
    let connection = Connection::from_socket(client_end)?;
    let mut queue: EventQueue<Offered> = connection.new_event_queue();
    connection.display().get_registry(&queue.handle(), ());
    let mut offered = Offered::default();
    for _ in 0..3 {
        queue.flush()?;
        display.dispatch_clients(&mut server)?;
        display.flush_clients()?;
        if let Some(guard) = queue.prepare_read() {
            let _ = guard.read();
        }
        queue.dispatch_pending(&mut offered)?;
    }
    Ok(offered.globals)
}

#[test]
fn a_privileged_client_is_offered_the_manager_at_version_1()
-> Result<(), Box<dyn std::error::Error>> {
    let globals = offered_globals(true)?;
    assert_eq!(globals, [("crownos_output_info_manager_v1".to_owned(), 1)]);
    Ok(())
}

#[test]
fn an_unprivileged_client_is_not_offered_the_manager() -> Result<(), Box<dyn std::error::Error>> {
    assert!(offered_globals(false)?.is_empty());
    Ok(())
}

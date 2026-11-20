//! `placements`: where the compositor put each window, once.

use anyhow::Context;
use crownos_protocols::window_control::v1::client::{
    crownos_window_control_manager_v1::{self, CrownosWindowControlManagerV1},
    crownos_window_v1::{self, CrownosWindowV1},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};

#[derive(Default)]
struct Placement {
    identifier: String,
    app_id: String,
    output: String,
    workspace: String,
}

#[derive(Default)]
struct Placements {
    windows: Vec<(CrownosWindowV1, Placement)>,
    complete: bool,
}

pub fn run() -> anyhow::Result<()> {
    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Placements>(&connection)?;
    globals
        .bind::<CrownosWindowControlManagerV1, _, _>(&queue.handle(), 1..=1, ())
        .context("the compositor does not trust this executable with window control")?;

    let mut placements = Placements::default();
    while !placements.complete {
        queue.blocking_dispatch(&mut placements)?;
    }
    queue.roundtrip(&mut placements)?;
    for (_, placement) in &placements.windows {
        println!(
            "window {} app_id={} output={} workspace={}",
            placement.identifier, placement.app_id, placement.output, placement.workspace
        );
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Placements {
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

impl Dispatch<CrownosWindowControlManagerV1, ()> for Placements {
    fn event(
        state: &mut Self,
        _: &CrownosWindowControlManagerV1,
        event: crownos_window_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            crownos_window_control_manager_v1::Event::Window { window } => {
                state.windows.push((window, Placement::default()))
            }
            crownos_window_control_manager_v1::Event::Done => state.complete = true,
            _ => {}
        }
    }

    event_created_child!(Placements, CrownosWindowControlManagerV1, [
        crownos_window_control_manager_v1::EVT_WINDOW_OPCODE => (CrownosWindowV1, ()),
    ]);
}

impl Dispatch<CrownosWindowV1, ()> for Placements {
    fn event(
        state: &mut Self,
        window: &CrownosWindowV1,
        event: crownos_window_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, placement)) = state.windows.iter_mut().find(|(known, _)| known == window)
        else {
            return;
        };
        match event {
            crownos_window_v1::Event::Identifier { identifier } => {
                placement.identifier = identifier
            }
            crownos_window_v1::Event::AppId { app_id } => placement.app_id = app_id,
            crownos_window_v1::Event::Placement {
                output, workspace, ..
            } => {
                placement.output = output;
                placement.workspace = workspace;
            }
            _ => {}
        }
    }
}

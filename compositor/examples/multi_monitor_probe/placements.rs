//! `placements`: where the compositor put each window, once.

use anyhow::Context;
use crownos_protocols::window_control::v1::client::{
    crownos_window_control_manager_v1::{self, CrownosWindowControlManagerV1},
    crownos_window_v1::{self, CrownosWindowV1},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};

#[derive(Default)]
pub struct Placement {
    pub identifier: String,
    pub app_id: String,
    output: String,
    workspace: String,
    rect: (i32, i32, i32, i32),
}

#[derive(Default)]
pub struct Placements {
    pub windows: Vec<(CrownosWindowV1, Placement)>,
    focused: Option<CrownosWindowV1>,
    complete: bool,
}

/// Binds window control at `version` and reads the window list once.
pub fn load(version: u32) -> anyhow::Result<(EventQueue<Placements>, Placements)> {
    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Placements>(&connection)?;
    globals
        .bind::<CrownosWindowControlManagerV1, _, _>(&queue.handle(), version..=version, ())
        .with_context(|| {
            format!("window control version {version} is not offered to this executable")
        })?;

    let mut placements = Placements::default();
    while !placements.complete {
        queue.blocking_dispatch(&mut placements)?;
    }
    // The directory catches up with the model only once someone is bound,
    // so the first answer can be one change behind; the second is current.
    queue.roundtrip(&mut placements)?;
    queue.roundtrip(&mut placements)?;
    Ok((queue, placements))
}

pub fn run() -> anyhow::Result<()> {
    let (_queue, placements) = load(1)?;
    for (window, placement) in &placements.windows {
        let keyboard = match placements.focused.as_ref() == Some(window) {
            true => " keyboard",
            false => "",
        };
        let (x, y, width, height) = placement.rect;
        println!(
            "window {} app_id={} output={} workspace={} rect={x},{y},{width}x{height}{keyboard}",
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
            crownos_window_control_manager_v1::Event::Focus { window } => state.focused = window,
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
                x,
                y,
                width,
                height,
                output,
                workspace,
            } => {
                placement.output = output;
                placement.workspace = workspace;
                placement.rect = (x, y, width, height);
            }
            _ => {}
        }
    }
}

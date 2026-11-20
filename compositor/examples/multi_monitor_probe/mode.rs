//! `mode [OUTPUT [tiling|floating]]`: the mode of the focused workspace and of
//! each output's active workspace through `crownos_workspace_mode_v1`
//! version 2, after switching OUTPUT's active workspace when a mode is given.

use anyhow::{Context, bail};
use crownos_protocols::workspace_mode::v1::client::{
    crownos_workspace_mode_manager_v1::{self, CrownosWorkspaceModeManagerV1, Mode},
    crownos_workspace_mode_output_v1::{self, CrownosWorkspaceModeOutputV1},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry},
};

const OUTPUT_VERSION: u32 = 4;
const MODE_VERSION: u32 = 2;

struct Watched {
    name: String,
    output: wl_output::WlOutput,
    object: Option<CrownosWorkspaceModeOutputV1>,
    mode: Option<Mode>,
}

#[derive(Default)]
struct Modes {
    focused: Option<Mode>,
    outputs: Vec<Watched>,
}

pub fn run(output: Option<&str>, mode: Option<&str>) -> anyhow::Result<()> {
    let requested = mode.map(parse_mode).transpose()?;
    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Modes>(&connection)?;
    let handle = queue.handle();
    let mut state = Modes::default();
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" && global.version >= OUTPUT_VERSION {
            let bound = globals.registry().bind::<wl_output::WlOutput, _, _>(
                global.name,
                OUTPUT_VERSION,
                &handle,
                (),
            );
            state.outputs.push(Watched {
                name: String::new(),
                output: bound,
                object: None,
                mode: None,
            });
        }
    }
    let manager: CrownosWorkspaceModeManagerV1 = globals
        .bind(&handle, MODE_VERSION..=MODE_VERSION, ())
        .context(
            "the compositor does not offer crownos_workspace_mode_v1 version 2 to this client",
        )?;
    queue.roundtrip(&mut state)?;

    for (index, watched) in state.outputs.iter_mut().enumerate() {
        watched.object = Some(manager.get_output(&watched.output, &handle, index));
    }
    queue.roundtrip(&mut state)?;

    if let (Some(name), Some(requested)) = (output, requested) {
        let object = state
            .outputs
            .iter()
            .find(|watched| watched.name == name)
            .and_then(|watched| watched.object.as_ref())
            .with_context(|| format!("no output named {name}"))?;
        object.set_mode(requested);
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;
    }

    println!("focused mode={}", describe(state.focused));
    for watched in &state.outputs {
        if output.is_none_or(|name| name == watched.name) {
            println!("output {} mode={}", watched.name, describe(watched.mode));
        }
    }
    Ok(())
}

fn parse_mode(mode: &str) -> anyhow::Result<Mode> {
    match mode {
        "tiling" => Ok(Mode::Tiling),
        "floating" => Ok(Mode::Floating),
        other => bail!("unknown mode {other}; expected tiling or floating"),
    }
}

fn describe(mode: Option<Mode>) -> &'static str {
    match mode {
        Some(Mode::Tiling) => "tiling",
        Some(Mode::Floating) => "floating",
        _ => "unknown",
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Modes {
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

impl Dispatch<wl_output::WlOutput, ()> for Modes {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some(watched) = state
                .outputs
                .iter_mut()
                .find(|watched| watched.output == *output)
        {
            watched.name = name;
        }
    }
}

impl Dispatch<CrownosWorkspaceModeManagerV1, ()> for Modes {
    fn event(
        state: &mut Self,
        _: &CrownosWorkspaceModeManagerV1,
        event: crownos_workspace_mode_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let crownos_workspace_mode_manager_v1::Event::Mode {
            mode: WEnum::Value(mode),
        } = event
        {
            state.focused = Some(mode);
        }
    }
}

impl Dispatch<CrownosWorkspaceModeOutputV1, usize> for Modes {
    fn event(
        state: &mut Self,
        _: &CrownosWorkspaceModeOutputV1,
        event: crownos_workspace_mode_output_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let crownos_workspace_mode_output_v1::Event::Mode {
            mode: WEnum::Value(mode),
        } = event
            && let Some(watched) = state.outputs.get_mut(*index)
        {
            watched.mode = Some(mode);
        }
    }
}

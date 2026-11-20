//! `brightness [OUTPUT [LEVEL[,LEVEL...]|refresh]]`: the control and level of
//! each output through `crownos_output_brightness_v1`. With LEVEL, sends one
//! `set_level` per value in one burst; with `refresh`, asks for a hardware
//! read. Then prints every event for two more seconds.

use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use crownos_protocols::output_brightness::v1::client::{
    crownos_output_brightness_manager_v1::CrownosOutputBrightnessManagerV1,
    crownos_output_brightness_v1::{self, Control, CrownosOutputBrightnessV1},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry},
};

const OUTPUT_VERSION: u32 = 4;
const BRIGHTNESS_VERSION: u32 = 1;
const LISTEN: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

pub enum Change {
    Levels(Vec<u32>),
    Refresh,
}

impl Change {
    pub fn parse(argument: &str) -> anyhow::Result<Self> {
        if argument == "refresh" {
            return Ok(Self::Refresh);
        }
        let levels = argument
            .split(',')
            .map(|level| {
                level
                    .parse()
                    .with_context(|| format!("{level} is no level"))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self::Levels(levels))
    }
}

struct Watched {
    name: String,
    output: wl_output::WlOutput,
    object: Option<CrownosOutputBrightnessV1>,
}

#[derive(Default)]
struct Outputs {
    watched: Vec<Watched>,
}

pub fn run(output: Option<&str>, change: Option<Change>) -> anyhow::Result<()> {
    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Outputs>(&connection)?;
    let handle = queue.handle();
    let manager: CrownosOutputBrightnessManagerV1 = globals
        .bind(&handle, BRIGHTNESS_VERSION..=BRIGHTNESS_VERSION, ())
        .context("the compositor does not offer crownos_output_brightness_v1 to this client")?;
    let mut state = Outputs::default();
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" && global.version >= OUTPUT_VERSION {
            let bound = globals.registry().bind::<wl_output::WlOutput, _, _>(
                global.name,
                OUTPUT_VERSION,
                &handle,
                (),
            );
            state.watched.push(Watched {
                name: String::new(),
                output: bound,
                object: None,
            });
        }
    }
    queue.roundtrip(&mut state)?;

    for (index, watched) in state.watched.iter_mut().enumerate() {
        if output.is_none_or(|name| name == watched.name) {
            watched.object = Some(manager.get_output(&watched.output, &handle, index));
        }
    }
    queue.roundtrip(&mut state)?;

    if let Some(change) = change {
        let Some(name) = output else {
            bail!("a level or refresh needs an output");
        };
        let object = state
            .watched
            .iter()
            .find(|watched| watched.name == name)
            .and_then(|watched| watched.object.as_ref())
            .with_context(|| format!("no output named {name}"))?;
        match change {
            Change::Levels(levels) => {
                for level in levels {
                    println!("output {name} set_level {level}");
                    object.set_level(level);
                }
            }
            Change::Refresh => {
                println!("output {name} refresh");
                object.refresh();
            }
        }
    }

    let deadline = Instant::now() + LISTEN;
    while Instant::now() < deadline {
        queue.flush()?;
        if let Some(guard) = queue.prepare_read() {
            let _ = guard.read();
        }
        queue.dispatch_pending(&mut state)?;
        std::thread::sleep(POLL_INTERVAL);
    }
    if let Some(error) = connection.protocol_error() {
        bail!("protocol error: {error}");
    }
    Ok(())
}

fn control_name(control: WEnum<Control>) -> String {
    match control {
        WEnum::Value(Control::None) => "none".to_owned(),
        WEnum::Value(Control::Backlight) => "backlight".to_owned(),
        WEnum::Value(Control::Ddc) => "ddc".to_owned(),
        WEnum::Value(Control::Software) => "software".to_owned(),
        WEnum::Value(other) => format!("{other:?}"),
        WEnum::Unknown(value) => format!("unknown({value})"),
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Outputs {
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

impl Dispatch<wl_output::WlOutput, ()> for Outputs {
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
                .watched
                .iter_mut()
                .find(|watched| watched.output == *output)
        {
            watched.name = name;
        }
    }
}

impl Dispatch<CrownosOutputBrightnessManagerV1, ()> for Outputs {
    fn event(
        _: &mut Self,
        _: &CrownosOutputBrightnessManagerV1,
        _: <CrownosOutputBrightnessManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<CrownosOutputBrightnessV1, usize> for Outputs {
    fn event(
        state: &mut Self,
        _: &CrownosOutputBrightnessV1,
        event: crownos_output_brightness_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let name = state
            .watched
            .get(*index)
            .map_or("?", |watched| watched.name.as_str());
        let line = match event {
            crownos_output_brightness_v1::Event::Control { control } => {
                format!("control={}", control_name(control))
            }
            crownos_output_brightness_v1::Event::Level { level } => format!("level={level}"),
            _ => return,
        };
        println!("output {name} {line}");
    }
}

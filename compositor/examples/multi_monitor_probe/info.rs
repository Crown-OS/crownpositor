//! `info [SECONDS]`: what `crownos_output_info_v1` says about each output,
//! one line per event, then every change for SECONDS more (2 by default).

use std::time::{Duration, Instant};

use anyhow::Context;
use crownos_protocols::output_info::v1::client::{
    crownos_output_info_manager_v1::CrownosOutputInfoManagerV1,
    crownos_output_info_v1::{self, CrownosOutputInfoV1},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry},
};

const OUTPUT_VERSION: u32 = 4;
const INFO_VERSION: u32 = 1;
const POLL_INTERVAL: Duration = Duration::from_millis(20);

struct Watched {
    name: String,
    output: wl_output::WlOutput,
}

struct Outputs {
    manager: CrownosOutputInfoManagerV1,
    watched: Vec<Watched>,
}

impl Outputs {
    fn watch(&mut self, registry: &wl_registry::WlRegistry, name: u32, handle: &QueueHandle<Self>) {
        let output = registry.bind::<wl_output::WlOutput, _, _>(name, OUTPUT_VERSION, handle, ());
        self.manager.get_output(&output, handle, self.watched.len());
        self.watched.push(Watched {
            name: format!("wl_output#{name}"),
            output,
        });
    }
}

pub fn run(seconds: u64) -> anyhow::Result<()> {
    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Outputs>(&connection)?;
    let handle = queue.handle();
    let manager: CrownosOutputInfoManagerV1 = globals
        .bind(&handle, INFO_VERSION..=INFO_VERSION, ())
        .context("the compositor does not offer crownos_output_info_v1 to this client")?;
    let mut state = Outputs {
        manager,
        watched: Vec::new(),
    };
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" && global.version >= OUTPUT_VERSION {
            state.watch(globals.registry(), global.name, &handle);
        }
    }
    queue.roundtrip(&mut state)?;

    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        queue.flush()?;
        if let Some(guard) = queue.prepare_read() {
            let _ = guard.read();
        }
        queue.dispatch_pending(&mut state)?;
        std::thread::sleep(POLL_INTERVAL);
    }
    Ok(())
}

fn flag_names<Flags>(value: WEnum<Flags>, names: impl Fn(Flags) -> Vec<&'static str>) -> String {
    match value {
        WEnum::Value(flags) => names(flags).join("|"),
        WEnum::Unknown(bits) => format!("unknown({bits:#x})"),
    }
}

fn shell_names(shell: crownos_output_info_v1::Shell) -> Vec<&'static str> {
    use crownos_output_info_v1::Shell;
    [
        (Shell::Bar, "bar"),
        (Shell::Dock, "dock"),
        (Shell::Notifications, "notifications"),
    ]
    .into_iter()
    .filter(|(bit, _)| shell.contains(*bit))
    .map(|(_, name)| name)
    .collect()
}

fn state_names(state: crownos_output_info_v1::State) -> Vec<&'static str> {
    use crownos_output_info_v1::State;
    [(State::Primary, "primary"), (State::Focused, "focused")]
        .into_iter()
        .filter(|(bit, _)| state.contains(*bit))
        .map(|(_, name)| name)
        .collect()
}

fn capability_names(capabilities: crownos_output_info_v1::Capability) -> Vec<&'static str> {
    use crownos_output_info_v1::Capability;
    [(Capability::Hdr, "hdr"), (Capability::Virtual, "virtual")]
        .into_iter()
        .filter(|(bit, _)| capabilities.contains(*bit))
        .map(|(_, name)| name)
        .collect()
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Outputs {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "wl_output"
            && version >= OUTPUT_VERSION
        {
            state.watch(registry, name, handle);
        }
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

impl Dispatch<CrownosOutputInfoManagerV1, ()> for Outputs {
    fn event(
        _: &mut Self,
        _: &CrownosOutputInfoManagerV1,
        _: <CrownosOutputInfoManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<CrownosOutputInfoV1, usize> for Outputs {
    fn event(
        state: &mut Self,
        _: &CrownosOutputInfoV1,
        event: crownos_output_info_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let name = state
            .watched
            .get(*index)
            .map_or("?", |watched| watched.name.as_str());
        let line = match event {
            crownos_output_info_v1::Event::Identity { identity } => {
                format!("identity={identity:?}")
            }
            crownos_output_info_v1::Event::Shell { shell } => {
                format!("shell={}", flag_names(shell, shell_names))
            }
            crownos_output_info_v1::Event::State { state } => {
                format!("state={}", flag_names(state, state_names))
            }
            crownos_output_info_v1::Event::Capabilities { capabilities } => {
                format!(
                    "capabilities={}",
                    flag_names(capabilities, capability_names)
                )
            }
            crownos_output_info_v1::Event::ScanoutDepth { bits } => {
                format!("scanout_depth={bits}")
            }
            crownos_output_info_v1::Event::Done => "done".to_owned(),
            _ => return,
        };
        println!("output {name} {line}");
    }
}

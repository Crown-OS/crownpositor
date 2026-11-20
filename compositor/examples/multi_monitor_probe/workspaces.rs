//! `workspaces`: each `ext_workspace` group, the outputs it is on and its
//! workspaces, the active one marked.

use anyhow::Context;
use wayland_client::{
    Connection, Dispatch, QueueHandle, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry},
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};

const OUTPUT_VERSION: u32 = 4;

#[derive(Default)]
struct Group {
    outputs: Vec<wl_output::WlOutput>,
    workspaces: Vec<ExtWorkspaceHandleV1>,
}

#[derive(Default)]
struct Workspace {
    id: String,
    name: String,
    active: bool,
}

#[derive(Default)]
struct Workspaces {
    output_names: Vec<(wl_output::WlOutput, String)>,
    groups: Vec<(ExtWorkspaceGroupHandleV1, Group)>,
    workspaces: Vec<(ExtWorkspaceHandleV1, Workspace)>,
}

impl Workspaces {
    fn output_name(&self, output: &wl_output::WlOutput) -> &str {
        self.output_names
            .iter()
            .find(|(known, _)| known == output)
            .map_or("?", |(_, name)| name.as_str())
    }
}

pub fn run() -> anyhow::Result<()> {
    let connection = Connection::connect_to_env().context("no Wayland compositor to talk to")?;
    let (globals, mut queue) = registry_queue_init::<Workspaces>(&connection)?;
    let handle = queue.handle();
    let mut state = Workspaces::default();
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" && global.version >= OUTPUT_VERSION {
            let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
                global.name,
                OUTPUT_VERSION,
                &handle,
                (),
            );
            state.output_names.push((output, String::new()));
        }
    }
    globals
        .bind::<ExtWorkspaceManagerV1, _, _>(&handle, 1..=1, ())
        .context("the compositor does not offer ext_workspace_v1")?;
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;

    for (_, group) in &state.groups {
        let outputs: Vec<&str> = group
            .outputs
            .iter()
            .map(|output| state.output_name(output))
            .collect();
        println!("group outputs={}", outputs.join(","));
        for handle in &group.workspaces {
            let Some((_, workspace)) = state.workspaces.iter().find(|(known, _)| known == handle)
            else {
                continue;
            };
            println!(
                "  workspace {} name={:?}{}",
                workspace.id,
                workspace.name,
                if workspace.active { " active" } else { "" }
            );
        }
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Workspaces {
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

impl Dispatch<wl_output::WlOutput, ()> for Workspaces {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some((_, known)) = state
                .output_names
                .iter_mut()
                .find(|(known, _)| known == output)
        {
            *known = name;
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for Workspaces {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group } => {
                state.groups.push((workspace_group, Group::default()))
            }
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                state.workspaces.push((workspace, Workspace::default()))
            }
            _ => {}
        }
    }

    event_created_child!(Workspaces, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Workspaces {
    fn event(
        state: &mut Self,
        group: &ExtWorkspaceGroupHandleV1,
        event: ext_workspace_group_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, group)) = state.groups.iter_mut().find(|(known, _)| known == group) else {
            return;
        };
        match event {
            ext_workspace_group_handle_v1::Event::OutputEnter { output } => {
                group.outputs.push(output)
            }
            ext_workspace_group_handle_v1::Event::OutputLeave { output } => {
                group.outputs.retain(|known| *known != output)
            }
            ext_workspace_group_handle_v1::Event::WorkspaceEnter { workspace } => {
                group.workspaces.push(workspace)
            }
            ext_workspace_group_handle_v1::Event::WorkspaceLeave { workspace } => {
                group.workspaces.retain(|known| *known != workspace)
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for Workspaces {
    fn event(
        state: &mut Self,
        workspace: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, workspace)) = state
            .workspaces
            .iter_mut()
            .find(|(known, _)| known == workspace)
        else {
            return;
        };
        match event {
            ext_workspace_handle_v1::Event::Id { id } => workspace.id = id,
            ext_workspace_handle_v1::Event::Name { name } => workspace.name = name,
            ext_workspace_handle_v1::Event::State { state } => {
                workspace.active = state
                    .into_result()
                    .is_ok_and(|state| state.contains(ext_workspace_handle_v1::State::Active))
            }
            _ => {}
        }
    }
}

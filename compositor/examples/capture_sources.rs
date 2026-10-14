//! Lists what a screencast picker can offer: every window from
//! `ext_foreign_toplevel_list_v1` and every workspace from `ext_workspace_v1`,
//! with the identifiers `crownos_screencast_v1`, `grim -T` and the portal
//! take. `--activate <workspace id>` switches to a workspace.
//!
//! ```text
//! cargo run -p compositor --example capture_sources
//! cargo run -p compositor --example capture_sources -- --activate crownos-workspace-3
//! ```

use wayland_client::{
    Connection, Dispatch, QueueHandle, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    workspace::v1::client::{
        ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
        ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
        ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
    },
};

#[derive(Default)]
struct Toplevel {
    identifier: String,
    title: String,
    app_id: String,
}

#[derive(Default)]
struct Workspace {
    handle: Option<ExtWorkspaceHandleV1>,
    id: String,
    name: String,
    active: bool,
}

#[derive(Default)]
struct Sources {
    toplevels: Vec<(ExtForeignToplevelHandleV1, Toplevel)>,
    workspaces: Vec<Workspace>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let activate = std::env::args()
        .skip_while(|arg| arg != "--activate")
        .nth(1);
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Sources>(&connection)?;
    let handle = queue.handle();
    globals.bind::<ExtForeignToplevelListV1, _, _>(&handle, 1..=1, ())?;
    let manager = globals.bind::<ExtWorkspaceManagerV1, _, _>(&handle, 1..=1, ())?;

    let mut sources = Sources::default();
    queue.roundtrip(&mut sources)?;
    queue.roundtrip(&mut sources)?;

    for (_, toplevel) in &sources.toplevels {
        println!(
            "window    {}  app_id={:?} title={:?}",
            toplevel.identifier, toplevel.app_id, toplevel.title
        );
    }
    for workspace in &sources.workspaces {
        println!(
            "workspace {}  name={:?}{}",
            workspace.id,
            workspace.name,
            if workspace.active { " active" } else { "" }
        );
    }

    if let Some(target) = activate {
        let workspace = sources
            .workspaces
            .iter()
            .find(|workspace| workspace.id == target)
            .and_then(|workspace| workspace.handle.as_ref())
            .ok_or("no workspace with that id")?;
        workspace.activate();
        manager.commit();
        queue.roundtrip(&mut sources)?;
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Sources {
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

impl Dispatch<ExtForeignToplevelListV1, ()> for Sources {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.push((toplevel, Toplevel::default()));
        }
    }

    event_created_child!(Sources, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Sources {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, toplevel)) = state
            .toplevels
            .iter_mut()
            .find(|(known, _)| known == handle)
        else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                toplevel.identifier = identifier
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => toplevel.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => toplevel.app_id = app_id,
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for Sources {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_workspace_manager_v1::Event::Workspace { workspace } = event {
            state.workspaces.push(Workspace {
                handle: Some(workspace),
                ..Workspace::default()
            });
        }
    }

    event_created_child!(Sources, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Sources {
    fn event(
        _: &mut Self,
        _: &ExtWorkspaceGroupHandleV1,
        _: ext_workspace_group_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for Sources {
    fn event(
        state: &mut Self,
        handle: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(workspace) = state
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.handle.as_ref() == Some(handle))
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

//! Output hotplug on a real `Shell`, without a backend.
//!
//! A `Tile` needs a live client surface, so these tests carry no windows:
//! they check what the shell does with outputs and their empty workspaces.

use anyhow::Context;
use smithay::{
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::Display,
    utils::{Logical, Point, Transform},
};

use config::Config;

use crate::{
    shell::{
        Shell,
        monitor::{Monitor, OutputDescriptor, Presence},
        output_kind::{NestedPersistence, OutputKind},
    },
    state::State,
    utils::id::WorkspaceId,
};

struct Fixture {
    display: Display<State>,
    config: Config,
    shell: Shell,
}

struct Head<'a> {
    name: &'a str,
    size: (i32, i32),
    kind: OutputKind,
    serial: Option<&'a str>,
}

impl<'a> Head<'a> {
    fn physical(name: &'a str, size: (i32, i32)) -> Self {
        Self {
            name,
            size,
            kind: OutputKind::Physical,
            serial: None,
        }
    }

    fn of_kind(self, kind: OutputKind) -> Self {
        Self { kind, ..self }
    }

    fn with_serial(self, serial: &'a str) -> Self {
        Self {
            serial: Some(serial),
            ..self
        }
    }
}

impl Fixture {
    fn new() -> anyhow::Result<Self> {
        Self::with_config(Config::default())
    }

    fn with_config(config: Config) -> anyhow::Result<Self> {
        let display = Display::<State>::new()?;
        let shell = Shell::try_new(&display.handle(), &config)?;
        Ok(Self {
            display,
            config,
            shell,
        })
    }

    fn plug(&mut self, name: &str, size: (i32, i32)) -> Output {
        self.plug_head(Head::physical(name, size))
    }

    fn plug_head(&mut self, head: Head) -> Output {
        let mode = Mode {
            size: head.size.into(),
            refresh: 60_000,
        };
        self.shell.add_output(
            &self.display.handle(),
            &self.config,
            OutputDescriptor {
                name: head.name.to_owned(),
                physical: PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Test".into(),
                    model: "Monitor".into(),
                    serial_number: head.name.to_owned(),
                },
                modes: vec![mode],
                preferred: Some(mode),
                current: mode,
                native_transform: Transform::Normal,
                refresh_interval: None,
                serial: head.serial.map(str::to_owned),
                edid: None,
                presence: Presence::Shown,
                kind: head.kind,
            },
        )
    }

    fn unplug(&mut self, output: &Output) {
        self.shell.remove_output(&self.display.handle(), output);
    }

    fn monitor(&self, output: &Output) -> anyhow::Result<&Monitor> {
        self.shell
            .monitor(output)
            .with_context(|| format!("{} is not in the shell", output.name()))
    }

    fn origin(&self, output: &Output) -> anyhow::Result<Point<i32, Logical>> {
        Ok(self.monitor(output)?.geometry().loc)
    }

    fn workspace_ids(&self, output: &Output) -> anyhow::Result<Vec<WorkspaceId>> {
        Ok(self
            .monitor(output)?
            .workspaces()
            .iter()
            .map(|workspace| workspace.id())
            .collect())
    }
}

#[test]
fn two_outputs_are_arranged_side_by_side() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let right = fixture.plug("DP-2", (1280, 720));

    assert_eq!(fixture.origin(&left)?, (0, 0).into());
    assert_eq!(fixture.origin(&right)?, (1920, 0).into());
    Ok(())
}

#[test]
fn removing_the_left_output_moves_the_right_one_to_the_origin() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let right = fixture.plug("DP-2", (1280, 720));

    fixture.unplug(&left);

    assert!(fixture.shell.monitor(&left).is_none());
    assert_eq!(fixture.origin(&right)?, (0, 0).into());
    Ok(())
}

#[test]
fn the_survivor_keeps_its_workspaces() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let right = fixture.plug("DP-2", (1280, 720));
    let before = fixture.workspace_ids(&right)?;

    fixture.unplug(&left);

    assert_eq!(fixture.workspace_ids(&right)?, before);
    Ok(())
}

#[test]
fn an_empty_workspace_is_not_carried_back_on_replug() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let right = fixture.plug("DP-2", (1280, 720));
    let left_before = fixture.workspace_ids(&left)?;
    let right_before = fixture.workspace_ids(&right)?;

    fixture.unplug(&left);
    let left_again = fixture.plug("DP-1", (1920, 1080));

    let left_after = fixture.workspace_ids(&left_again)?;
    assert_eq!(fixture.workspace_ids(&right)?, right_before);
    assert_eq!(left_after.len(), 1);
    assert!(left_after.iter().all(|id| !left_before.contains(id)));
    Ok(())
}

fn identity_of(fixture: &Fixture, output: &Output) -> anyhow::Result<String> {
    Ok(fixture
        .monitor(output)?
        .config()
        .connector
        .as_str()
        .to_owned())
}

#[test]
fn twin_monitors_go_by_their_connectors_until_one_leaves() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let first = fixture.plug_head(Head::physical("DP-1", (1920, 1080)).with_serial("A1"));
    assert_eq!(identity_of(&fixture, &first)?, "Test Monitor A1");

    let twin = fixture.plug_head(Head::physical("DP-2", (1920, 1080)).with_serial("A1"));
    assert_eq!(identity_of(&fixture, &first)?, "DP-1");
    assert_eq!(identity_of(&fixture, &twin)?, "DP-2");

    fixture.unplug(&first);
    assert_eq!(identity_of(&fixture, &twin)?, "Test Monitor A1");
    Ok(())
}

#[test]
fn only_physical_monitors_are_persisted_unless_nested_is_asked_for() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.plug("DP-1", (1920, 1080));
    fixture.plug_head(Head::physical("winit-1", (1280, 720)).of_kind(OutputKind::Nested));
    fixture.plug_head(Head::physical("VIRTUAL-1", (1280, 720)).of_kind(OutputKind::Virtual));
    fixture.plug_head(Head::physical("HEADLESS-1", (3840, 2160)).of_kind(OutputKind::Virtual));

    let names = |nested| -> Vec<String> {
        fixture
            .shell
            .persisted_monitors(nested)
            .map(|monitor| monitor.config().name.clone())
            .collect()
    };
    assert_eq!(names(NestedPersistence::Skip), ["DP-1"]);
    assert_eq!(names(NestedPersistence::Save), ["DP-1", "winit-1"]);
    Ok(())
}

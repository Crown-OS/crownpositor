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
    },
    state::State,
    utils::id::WorkspaceId,
};

struct Fixture {
    display: Display<State>,
    shell: Shell,
}

impl Fixture {
    fn new() -> anyhow::Result<Self> {
        let display = Display::<State>::new()?;
        let shell = Shell::try_new(&display.handle(), &Config::default())?;
        Ok(Self { display, shell })
    }

    fn plug(&mut self, name: &str, size: (i32, i32)) -> Output {
        let mode = Mode {
            size: size.into(),
            refresh: 60_000,
        };
        self.shell.add_output(
            &self.display.handle(),
            &Config::default(),
            OutputDescriptor {
                name: name.to_owned(),
                physical: PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Test".into(),
                    model: "Monitor".into(),
                    serial_number: name.to_owned(),
                },
                modes: vec![mode],
                preferred: Some(mode),
                current: mode,
                native_transform: Transform::Normal,
                refresh_interval: None,
                serial: None,
                edid: None,
                presence: Presence::Shown,
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

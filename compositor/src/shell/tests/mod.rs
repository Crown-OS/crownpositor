//! Output hotplug on a real `Shell`, without a backend.
//!
//! A `Tile` needs a live client surface, so these tests carry no windows:
//! they check what the shell does with outputs and their empty workspaces.

mod workspace_scope;

use anyhow::Context;
use smithay::{
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::Display,
    utils::{Logical, Point, Transform},
};

use config::Config;

use crate::{
    layout::{Gaps, WorkspaceMode},
    shell::{
        Location, Shell,
        identity::OfflineHead,
        monitor::{Monitor, OutputDescriptor, Presence},
        output_kind::{NestedPersistence, OutputKind},
        stash::STASH,
        workspace::Workspace,
    },
    state::State,
    utils::id::{OutputId, WorkspaceId},
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
    presence: Presence,
}

impl<'a> Head<'a> {
    fn physical(name: &'a str, size: (i32, i32)) -> Self {
        Self {
            name,
            size,
            kind: OutputKind::Physical,
            serial: None,
            presence: Presence::Shown,
        }
    }

    fn hidden(self) -> Self {
        Self {
            presence: Presence::Hidden,
            ..self
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
                presence: head.presence,
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
fn a_switched_off_twin_still_makes_both_go_by_their_connectors() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let lit = fixture.plug_head(Head::physical("DP-1", (1920, 1080)).with_serial("A1"));
    fixture.shell.set_offline_heads(vec![OfflineHead::new(
        "DP-2",
        "Test Monitor A1",
        OutputKind::Physical,
    )]);
    assert_eq!(identity_of(&fixture, &lit)?, "DP-1");
    assert_eq!(fixture.shell.offline_heads()[0].identity, "DP-2");

    let switched_on = fixture.plug_head(Head::physical("DP-2", (1920, 1080)).with_serial("A1"));
    assert_eq!(identity_of(&fixture, &switched_on)?, "DP-2");
    assert!(fixture.shell.offline_heads().is_empty());

    fixture.shell.set_offline_heads(Vec::new());
    fixture.unplug(&switched_on);
    assert_eq!(identity_of(&fixture, &lit)?, "Test Monitor A1");
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

fn focused_name(fixture: &Fixture) -> Option<String> {
    fixture
        .shell
        .focused_monitor()
        .map(|monitor| monitor.config().name.clone())
}

fn keyboard_name(fixture: &Fixture) -> Option<String> {
    fixture
        .shell
        .keyboard_monitor()
        .map(|monitor| monitor.config().name.clone())
}

#[test]
fn the_pointer_moves_the_focused_monitor_and_leaves_the_keyboard() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.plug("DP-1", (1920, 1080));
    fixture.plug("DP-2", (1280, 720));
    assert_eq!(focused_name(&fixture).as_deref(), Some("DP-1"));
    assert_eq!(keyboard_name(&fixture).as_deref(), Some("DP-1"));

    assert!(fixture.shell.focus_at((2000.0, 100.0).into()));
    assert_eq!(focused_name(&fixture).as_deref(), Some("DP-2"));
    assert_eq!(keyboard_name(&fixture).as_deref(), Some("DP-1"));

    assert!(fixture.shell.focus_keyboard_on_focused());
    assert_eq!(keyboard_name(&fixture).as_deref(), Some("DP-2"));
    Ok(())
}

#[test]
fn a_point_off_every_monitor_keeps_the_focused_one() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.plug("DP-1", (1920, 1080));
    fixture.plug("DP-2", (1280, 720));
    fixture.shell.focus_at((2000.0, 100.0).into());

    assert!(!fixture.shell.focus_at((2000.0, 900.0).into()));
    assert_eq!(focused_name(&fixture).as_deref(), Some("DP-2"));
    Ok(())
}

#[test]
fn unplugging_the_focused_monitor_hands_both_focuses_on() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let middle = fixture.plug("DP-2", (1920, 1080));
    fixture.plug("DP-3", (1280, 720));
    fixture.shell.focus_at((2000.0, 100.0).into());
    fixture.shell.focus_keyboard_on_focused();

    fixture.unplug(&middle);
    fixture.shell.refresh();

    assert_eq!(focused_name(&fixture).as_deref(), Some("DP-3"));
    assert_eq!(keyboard_name(&fixture).as_deref(), Some("DP-3"));
    fixture.unplug(&left);
    assert_eq!(focused_name(&fixture).as_deref(), Some("DP-3"));
    assert_eq!(keyboard_name(&fixture).as_deref(), Some("DP-3"));
    Ok(())
}

#[test]
fn the_nearest_monitor_survives_when_the_focused_one_is_unplugged() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let middle = fixture.plug("DP-2", (1920, 1080));
    let right = fixture.plug("DP-3", (1920, 1080));
    fixture.shell.focus_at((4000.0, 100.0).into());
    fixture.shell.focus_keyboard_on_focused();

    fixture.unplug(&right);
    assert_eq!(focused_name(&fixture).as_deref(), Some("DP-2"));
    assert_eq!(keyboard_name(&fixture).as_deref(), Some("DP-2"));

    fixture.unplug(&middle);
    assert_eq!(focused_name(&fixture).as_deref(), Some("DP-1"));
    assert!(fixture.shell.monitor(&left).is_some());
    Ok(())
}

#[test]
fn a_stashed_workspace_is_found_while_it_waits_and_comes_back_in_place() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let only = fixture.plug("DP-1", (1920, 1080));
    fixture.unplug(&only);
    assert!(fixture.shell.monitors().is_empty());
    assert_eq!(focused_name(&fixture), None);

    let waiting = Workspace::new(OutputId::next(), WorkspaceMode::Floating, Gaps::default());
    let waiting_id = waiting.id();
    fixture.shell.stash(vec![waiting]);
    let stashed_at = Location {
        output: STASH,
        workspace: waiting_id,
    };
    assert!(fixture.shell.workspace(stashed_at).is_some());

    let back = fixture.plug("DP-1", (1920, 1080));
    assert_eq!(fixture.workspace_ids(&back)?, [waiting_id]);
    assert!(fixture.shell.workspace(stashed_at).is_none());
    assert_eq!(focused_name(&fixture).as_deref(), Some("DP-1"));
    Ok(())
}

#[test]
fn the_keyboard_never_goes_to_a_hidden_monitor() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.plug("DP-1", (1920, 1080));
    let hidden = fixture.plug_head(
        Head::physical("HEADLESS-1", (3840, 2160))
            .of_kind(OutputKind::Virtual)
            .hidden(),
    );

    let hidden_id = fixture.monitor(&hidden)?.id();
    assert!(!fixture.shell.focus_keyboard_on(hidden_id));
    assert_eq!(keyboard_name(&fixture).as_deref(), Some("DP-1"));
    Ok(())
}

fn primary_name(fixture: &Fixture) -> Option<String> {
    fixture
        .shell
        .primary()
        .map(|monitor| monitor.config().name.clone())
}

fn configured_primary(primary_output: &str) -> Config {
    let mut config = Config::default();
    config.compositor.primary_output = Some(primary_output.to_owned());
    config
}

#[test]
fn the_configured_primary_wins_by_connector_or_identity() -> anyhow::Result<()> {
    let mut by_connector = Fixture::with_config(configured_primary("HDMI-A-1"))?;
    by_connector.plug("eDP-1", (1920, 1200));
    by_connector.plug("HDMI-A-1", (2560, 1440));
    assert_eq!(primary_name(&by_connector).as_deref(), Some("HDMI-A-1"));

    let mut by_identity = Fixture::with_config(configured_primary("Test Monitor S1"))?;
    by_identity.plug("eDP-1", (1920, 1200));
    by_identity.plug_head(Head::physical("DP-3", (2560, 1440)).with_serial("S1"));
    assert_eq!(primary_name(&by_identity).as_deref(), Some("DP-3"));
    Ok(())
}

#[test]
fn the_internal_panel_is_primary_when_nothing_is_configured() -> anyhow::Result<()> {
    let mut fixture = Fixture::with_config(configured_primary("DP-9"))?;
    fixture.plug("HDMI-A-1", (2560, 1440));
    fixture.plug("eDP-1", (1920, 1200));
    assert_eq!(primary_name(&fixture).as_deref(), Some("eDP-1"));
    Ok(())
}

#[test]
fn the_top_left_monitor_is_primary_without_a_panel() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.plug("DP-2", (2560, 1440));
    fixture.plug("DP-1", (1920, 1080));
    assert_eq!(primary_name(&fixture).as_deref(), Some("DP-2"));

    fixture.shell.set_primary_output(Some("DP-1".to_owned()));
    assert_eq!(primary_name(&fixture).as_deref(), Some("DP-1"));
    Ok(())
}

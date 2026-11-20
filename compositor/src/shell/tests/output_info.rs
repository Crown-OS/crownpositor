//! What `crownos_output_info_v1` would tell a client about each monitor.

use anyhow::Context;
use config::{Compositor, Config, OutputSetting};
use protocols::crownos_output_info::{
    Capability, Field, OutputInfo, Shell as ShellSurfaces, State,
};
use smithay::output::Output;

use super::{Fixture, Head};
use crate::shell::output_kind::OutputKind;

fn info(fixture: &Fixture, output: &Output) -> anyhow::Result<OutputInfo> {
    let primary = fixture.shell.primary().map(|monitor| monitor.id());
    fixture
        .shell
        .output_info(fixture.monitor(output)?, primary, &fixture.config)
        .with_context(|| format!("{} has no identity", output.name()))
}

#[test]
fn twins_are_reported_under_their_connectors() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let first = fixture.plug_head(Head::physical("DP-1", (1920, 1080)).with_serial("A1"));
    let before = info(&fixture, &first)?;
    assert_eq!(before.identity, "Test Monitor A1");

    let twin = fixture.plug_head(Head::physical("DP-2", (1920, 1080)).with_serial("A1"));
    let after = info(&fixture, &first)?;
    assert_eq!(after.identity, "DP-1");
    assert_eq!(info(&fixture, &twin)?.identity, "DP-2");
    assert!(
        after
            .changed_since(Some(&before))
            .any(|field| field == Field::Identity)
    );
    Ok(())
}

#[test]
fn a_hidden_output_shows_no_shell_surface() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let shown = fixture.plug("DP-1", (1920, 1080));
    let hidden = fixture.plug_head(
        Head::physical("HEADLESS-1", (1920, 1080))
            .of_kind(OutputKind::Virtual)
            .hidden(),
    );

    let hidden = info(&fixture, &hidden)?;
    assert_eq!(hidden.shell, ShellSurfaces::empty());
    assert_eq!(hidden.state, State::empty());
    assert_eq!(hidden.capabilities, Capability::Virtual);
    assert_eq!(
        info(&fixture, &shown)?.shell,
        ShellSurfaces::Bar | ShellSurfaces::Dock | ShellSurfaces::Notifications
    );
    Ok(())
}

#[test]
fn a_new_primary_changes_the_state_of_both_outputs() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let right = fixture.plug("DP-2", (1920, 1080));
    let (left_before, right_before) = (info(&fixture, &left)?, info(&fixture, &right)?);
    assert!(left_before.state.contains(State::Primary));
    assert!(!right_before.state.contains(State::Primary));

    fixture.shell.set_primary_output(Some("DP-2".to_owned()));

    let (left_after, right_after) = (info(&fixture, &left)?, info(&fixture, &right)?);
    assert!(!left_after.state.contains(State::Primary));
    assert!(right_after.state.contains(State::Primary));
    for (before, after) in [(&left_before, &left_after), (&right_before, &right_after)] {
        let changed: Vec<Field> = after.changed_since(Some(before)).collect();
        assert!(changed.contains(&Field::State), "{changed:?}");
        assert!(
            changed.contains(&Field::Shell),
            "the dock follows the primary"
        );
    }
    Ok(())
}

#[test]
fn the_settings_choose_the_shell_surfaces() -> anyhow::Result<()> {
    let config = Config {
        compositor: Compositor {
            outputs: vec![OutputSetting {
                name: "DP-1".to_owned(),
                bar: Some(false),
                ..OutputSetting::default()
            }],
            ..Compositor::default()
        },
        ..Config::default()
    };
    let mut fixture = Fixture::with_config(config)?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let right = fixture.plug("DP-2", (1920, 1080));

    assert_eq!(
        info(&fixture, &left)?.shell,
        ShellSurfaces::Dock | ShellSurfaces::Notifications
    );
    assert_eq!(info(&fixture, &right)?.shell, ShellSurfaces::Bar);
    Ok(())
}

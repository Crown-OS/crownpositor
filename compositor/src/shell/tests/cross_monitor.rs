//! Snap edges and on-screen placement across a row and an L of monitors.

use anyhow::Context;
use crownos_config::schema::WorkspaceScope;
use smithay::output::Output;

use config::Config;

use super::{Fixture, Head};
use crate::{
    layout::Edges,
    shell::{Location, output_kind::OutputKind},
    utils::id::OutputId,
};

fn row_of_three(fixture: &mut Fixture) -> [Output; 3] {
    [
        fixture.plug("DP-1", (1920, 1080)),
        fixture.plug("DP-2", (2560, 1440)),
        fixture.plug("DP-3", (1280, 720)),
    ]
}

fn id(fixture: &Fixture, output: &Output) -> anyhow::Result<OutputId> {
    Ok(fixture.monitor(output)?.id())
}

fn edges(left: bool, right: bool, top: bool) -> Edges {
    Edges { left, right, top }
}

#[test]
fn a_row_of_three_snaps_only_at_its_outer_edges() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let [left, middle, right] = row_of_three(&mut fixture);
    let hidden = fixture.plug_head(
        Head::physical("HEADLESS-1", (1920, 1080))
            .of_kind(OutputKind::Virtual)
            .hidden(),
    );

    let shell = &fixture.shell;
    assert_eq!(
        shell.open_edges(id(&fixture, &left)?),
        edges(true, false, true)
    );
    assert_eq!(
        shell.open_edges(id(&fixture, &middle)?),
        edges(false, false, true)
    );
    assert_eq!(
        shell.open_edges(id(&fixture, &right)?),
        edges(false, true, true),
        "the hidden monitor parked to the right is no neighbour"
    );
    assert_eq!(shell.open_edges(id(&fixture, &hidden)?), Edges::ALL);
    Ok(())
}

#[test]
fn an_l_shape_closes_the_top_below_a_monitor_and_the_side_beside_one() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let [panel, above, beside] = [
        fixture.plug("eDP-1", (1920, 1080)),
        fixture.plug("DP-1", (1920, 1080)),
        fixture.plug("DP-2", (2560, 1440)),
    ];
    for (output, position) in [
        (&panel, (0, 1080)),
        (&above, (0, 0)),
        (&beside, (1920, 1080)),
    ] {
        fixture
            .shell
            .monitor_mut(output)
            .context("plugged")?
            .pin_position(position.into());
    }
    fixture.shell.arrange_outputs();

    let shell = &fixture.shell;
    assert_eq!(
        shell.open_edges(id(&fixture, &panel)?),
        edges(true, false, false)
    );
    assert_eq!(shell.open_edges(id(&fixture, &above)?), Edges::ALL);
    assert_eq!(
        shell.open_edges(id(&fixture, &beside)?),
        edges(false, true, true)
    );
    Ok(())
}

#[test]
fn only_an_active_workspace_on_a_shown_monitor_is_on_screen() -> anyhow::Result<()> {
    let mut config = Config::default();
    config.compositor.workspace_scope = WorkspaceScope::PerMonitor;
    let mut fixture = Fixture::with_config(config)?;
    let [_, middle, _] = row_of_three(&mut fixture);
    let hidden = fixture.plug_head(
        Head::physical("HEADLESS-1", (1920, 1080))
            .of_kind(OutputKind::Virtual)
            .hidden(),
    );
    let middle = id(&fixture, &middle)?;
    let hidden = id(&fixture, &hidden)?;

    let active = fixture
        .shell
        .active_location_on(middle)
        .context("a shown monitor has an active workspace")?;
    let inactive = Location {
        output: middle,
        workspace: fixture
            .shell
            .ensure_workspace(middle, 2)
            .context("the workspace is created")?,
    };

    assert!(fixture.shell.is_on_screen(active));
    assert!(!fixture.shell.is_on_screen(inactive));
    assert_eq!(fixture.shell.active_location_on(hidden), None);
    Ok(())
}

//! The overview across three shown monitors and a hidden one: all of them
//! together in `Shared` scope, the monitor under the pointer alone in
//! `PerMonitor` scope.

use anyhow::Context;
use crownos_config::schema::WorkspaceScope;
use smithay::output::Output;

use config::Config;

use super::{Fixture, Head};
use spacecontrol::scene::centre;

use crate::{
    shell::{output_kind::OutputKind, overview::SpaceControl},
    utils::id::OutputId,
};

struct Desk {
    fixture: Fixture,
    outputs: [Output; 4],
}

fn desk(scope: WorkspaceScope) -> anyhow::Result<Desk> {
    let mut config = Config::default();
    config.compositor.workspace_scope = scope;
    let mut fixture = Fixture::with_config(config)?;
    let outputs = [
        fixture.plug("DP-1", (1920, 1080)),
        fixture.plug("DP-2", (1920, 1080)),
        fixture.plug("DP-3", (1280, 720)),
        fixture.plug_head(
            Head::physical("HEADLESS-1", (3840, 2160))
                .of_kind(OutputKind::Virtual)
                .hidden(),
        ),
    ];
    Ok(Desk { fixture, outputs })
}

impl Desk {
    fn id(&self, index: usize) -> anyhow::Result<OutputId> {
        Ok(self.fixture.monitor(&self.outputs[index])?.id())
    }

    fn each_overview<T>(&self, read: impl Fn(&SpaceControl) -> T) -> anyhow::Result<Vec<T>> {
        self.outputs
            .iter()
            .map(|output| Ok(read(self.fixture.monitor(output)?.spacecontrol())))
            .collect()
    }

    fn open(&self) -> anyhow::Result<Vec<bool>> {
        self.each_overview(SpaceControl::is_open)
    }

    fn point_on_middle_monitor(&mut self) {
        self.fixture.shell.focus_at((2000.0, 100.0).into());
    }
}

#[test]
fn a_shared_overview_opens_and_closes_on_every_shown_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    desk.point_on_middle_monitor();

    desk.fixture.shell.toggle_overview();
    assert_eq!(desk.open()?, [true, true, true, false]);

    desk.fixture.shell.toggle_overview();
    assert_eq!(desk.open()?, [false, false, false, false]);
    Ok(())
}

#[test]
fn a_per_monitor_overview_opens_on_the_focused_monitor_alone() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    desk.point_on_middle_monitor();

    desk.fixture.shell.open_overview();
    assert_eq!(desk.open()?, [false, true, false, false]);

    desk.fixture.shell.close_overviews();
    assert_eq!(desk.open()?, [false, false, false, false]);
    Ok(())
}

#[test]
fn closing_leaves_no_overview_open_wherever_the_pointer_went() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    desk.fixture.shell.open_overview();
    desk.point_on_middle_monitor();

    desk.fixture.shell.toggle_overview();
    assert_eq!(desk.open()?, [false, false, false, false]);
    Ok(())
}

#[test]
fn a_shared_swipe_drives_every_shown_monitor_together() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;

    desk.fixture.shell.drive_overview(1.0);
    assert_eq!(
        desk.each_overview(SpaceControl::is_dragging)?,
        [true, true, true, false]
    );

    assert!(desk.fixture.shell.release_overview(false, 0.0));
    assert_eq!(desk.open()?, [true, true, true, false]);
    assert!(!desk.fixture.shell.release_overview(false, 0.0));
    Ok(())
}

#[test]
fn a_per_monitor_swipe_drives_the_focused_monitor_alone() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    desk.point_on_middle_monitor();

    desk.fixture.shell.drive_overview(1.0);
    assert!(desk.fixture.shell.release_overview(false, 0.0));
    assert_eq!(desk.open()?, [false, true, false, false]);
    Ok(())
}

#[test]
fn a_new_shared_workspace_shows_in_every_overview() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    desk.fixture.shell.open_overview();
    let middle = desk.id(1)?;

    desk.fixture.shell.create_workspace(middle);
    desk.fixture.shell.resolve_linked_overviews(middle);

    assert_eq!(desk.each_overview(|space| space.bar().len())?, [2, 2, 2, 0]);
    Ok(())
}

#[test]
fn a_click_on_one_shared_overview_closes_them_all() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    desk.fixture.shell.open_overview();

    desk.fixture.shell.close_linked_overviews(desk.id(2)?);
    assert_eq!(desk.open()?, [false, false, false, false]);
    Ok(())
}

#[test]
fn a_press_keeps_the_pointer_on_its_overview_off_its_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    desk.fixture.shell.open_overview();
    let middle = desk.id(1)?;
    let origin = desk
        .fixture
        .monitor(&desk.outputs[1])?
        .geometry()
        .loc
        .to_f64();
    let on_right = (4000.0, 100.0).into();
    assert_eq!(
        desk.fixture
            .shell
            .overview_under(on_right)
            .map(|(id, _)| id),
        Some(desk.id(2)?)
    );

    let preview = desk
        .fixture
        .monitor(&desk.outputs[1])?
        .spacecontrol()
        .bar()
        .first()
        .map(|slot| centre(slot.hit()))
        .context("the middle overview has no preview")?;
    let pressed = desk
        .fixture
        .shell
        .monitor_by_id_mut(middle)
        .and_then(|monitor| monitor.spacecontrol_mut().press(preview));
    assert!(pressed.is_some(), "the press missed the preview");

    assert_eq!(
        desk.fixture.shell.overview_under(on_right),
        Some((middle, on_right - origin))
    );
    Ok(())
}

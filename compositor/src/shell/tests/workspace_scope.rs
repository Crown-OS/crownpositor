//! Shared and per-monitor workspace scope across three monitors and a hidden
//! one.

use anyhow::Context;
use crownos_config::schema::WorkspaceScope;
use smithay::output::Output;

use config::Config;

use super::{Fixture, Head};
use crate::{
    handlers::ext_workspace::workspace_model,
    layout::{Direction, WorkspaceMode},
    shell::{output_kind::OutputKind, workspace::WorkspaceRef},
    utils::id::OutputId,
};

struct Desk {
    fixture: Fixture,
    shown: [Output; 3],
    hidden: Output,
}

fn desk(scope: WorkspaceScope) -> anyhow::Result<Desk> {
    let mut config = Config::default();
    config.compositor.workspace_scope = scope;
    let mut fixture = Fixture::with_config(config)?;
    let shown = [
        fixture.plug("DP-1", (1920, 1080)),
        fixture.plug("DP-2", (1920, 1080)),
        fixture.plug("DP-3", (1280, 720)),
    ];
    let hidden = fixture.plug_head(
        Head::physical("HEADLESS-1", (3840, 2160))
            .of_kind(OutputKind::Virtual)
            .hidden(),
    );
    Ok(Desk {
        fixture,
        shown,
        hidden,
    })
}

impl Desk {
    fn id(&self, output: &Output) -> anyhow::Result<OutputId> {
        Ok(self.fixture.monitor(output)?.id())
    }

    fn focused(&self) -> anyhow::Result<OutputId> {
        self.id(&self.shown[0])
    }

    /// `(workspace count, active index)` of each shown monitor, then the
    /// hidden one.
    fn shape(&self) -> anyhow::Result<Vec<(usize, usize)>> {
        self.shown
            .iter()
            .chain(std::iter::once(&self.hidden))
            .map(|output| {
                let monitor = self.fixture.monitor(output)?;
                Ok((monitor.workspaces().len(), monitor.active_index()))
            })
            .collect()
    }

    fn refresh(&mut self) {
        self.fixture.shell.refresh();
    }
}

#[test]
fn a_shared_switch_moves_every_shown_monitor_and_leaves_the_hidden_one() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;

    assert_eq!(desk.fixture.shell.create_workspace(focused), Some(1));
    assert!(desk.fixture.shell.switch_workspace(WorkspaceRef::Index(1)));
    desk.refresh();

    assert_eq!(desk.shape()?, [(2, 1), (2, 1), (2, 1), (1, 0)]);
    Ok(())
}

#[test]
fn a_per_monitor_switch_moves_the_focused_monitor_alone() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    let focused = desk.focused()?;

    desk.fixture.shell.create_workspace(focused);
    desk.fixture.shell.switch_workspace(WorkspaceRef::Index(1));
    desk.refresh();

    assert_eq!(desk.shape()?, [(2, 1), (1, 0), (1, 0), (1, 0)]);
    Ok(())
}

#[test]
fn ensuring_a_shared_workspace_pads_every_shown_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let middle = desk.id(&desk.shown[1])?;

    let ensured = desk.fixture.shell.ensure_workspace(middle, 3);
    desk.refresh();

    assert_eq!(
        ensured,
        Some(desk.fixture.monitor(&desk.shown[1])?.workspaces()[3].id())
    );
    assert_eq!(desk.shape()?, [(4, 0), (4, 0), (4, 0), (1, 0)]);
    Ok(())
}

#[test]
fn removing_a_shared_workspace_removes_its_index_everywhere() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;
    desk.fixture.shell.ensure_workspace(focused, 2);
    desk.fixture.shell.activate_index(focused, 2);
    let doomed = desk.fixture.monitor(&desk.shown[0])?.workspaces()[2].id();

    assert!(desk.fixture.shell.remove_workspace(focused, doomed));
    desk.refresh();

    assert_eq!(desk.shape()?, [(2, 1), (2, 1), (2, 1), (1, 0)]);
    Ok(())
}

#[test]
fn the_last_workspace_is_never_removed() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;
    let only = desk.fixture.monitor(&desk.shown[0])?.workspaces()[0].id();

    assert!(!desk.fixture.shell.remove_workspace(focused, only));
    desk.refresh();
    assert_eq!(desk.shape()?, [(1, 0), (1, 0), (1, 0), (1, 0)]);
    Ok(())
}

#[test]
fn moving_a_shared_workspace_reorders_every_shown_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;
    desk.fixture.shell.ensure_workspace(focused, 1);
    let order = |desk: &Desk, output: usize| -> anyhow::Result<Vec<_>> {
        desk.fixture.workspace_ids(&desk.shown[output])
    };
    let before: Vec<_> = (0..3)
        .map(|output| order(&desk, output))
        .collect::<Result<_, _>>()?;

    assert!(desk.fixture.shell.move_workspace(focused, 0, 1));
    desk.refresh();

    for (output, mut expected) in before.into_iter().enumerate() {
        expected.reverse();
        assert_eq!(order(&desk, output)?, expected);
    }
    Ok(())
}

#[test]
fn tearing_past_the_end_adds_a_workspace_on_every_shown_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;

    desk.fixture.shell.begin_workspace_swipe();
    desk.fixture.shell.update_workspace_swipe(-1.0);
    desk.fixture.shell.update_workspace_swipe(-2.0);
    assert!(desk.fixture.shell.end_workspace_swipe(0.0));
    desk.refresh();

    assert_eq!(desk.shape()?, [(2, 1), (2, 1), (2, 1), (1, 0)]);
    Ok(())
}

#[test]
fn tearing_past_the_start_puts_a_workspace_first_on_every_shown_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let first = desk.fixture.workspace_ids(&desk.shown[1])?;

    desk.fixture.shell.begin_workspace_swipe();
    desk.fixture.shell.update_workspace_swipe(1.0);
    desk.fixture.shell.update_workspace_swipe(2.0);
    assert!(desk.fixture.shell.end_workspace_swipe(0.0));
    desk.refresh();

    assert_eq!(desk.shape()?, [(2, 0), (2, 0), (2, 0), (1, 0)]);
    assert_eq!(desk.fixture.workspace_ids(&desk.shown[1])?[1], first[0]);
    Ok(())
}

#[test]
fn turning_shared_scope_on_pads_and_follows_the_focused_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    let focused = desk.focused()?;
    let last = desk.id(&desk.shown[2])?;
    desk.fixture.shell.ensure_workspace(focused, 2);
    desk.fixture.shell.activate_index(focused, 1);
    desk.fixture.shell.ensure_workspace(last, 3);
    desk.fixture.shell.activate_index(last, 3);
    desk.refresh();
    assert_eq!(desk.shape()?, [(3, 1), (1, 0), (4, 3), (1, 0)]);

    desk.fixture
        .shell
        .set_workspace_scope(WorkspaceScope::Shared);
    desk.refresh();
    assert_eq!(desk.shape()?, [(4, 1), (4, 1), (4, 1), (1, 0)]);

    desk.fixture
        .shell
        .set_workspace_scope(WorkspaceScope::PerMonitor);
    desk.refresh();
    assert_eq!(desk.shape()?, [(4, 1), (4, 1), (4, 1), (1, 0)]);
    desk.fixture.shell.switch_workspace(WorkspaceRef::Index(3));
    desk.refresh();
    assert_eq!(desk.shape()?, [(4, 3), (4, 1), (4, 1), (1, 0)]);
    Ok(())
}

#[test]
fn a_shared_mode_change_reaches_the_same_index_on_every_shown_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let hidden_mode = desk.fixture.monitor(&desk.hidden)?.active().mode();

    desk.fixture.shell.set_workspace_mode(hidden_mode.toggled());
    desk.refresh();

    for output in &desk.shown {
        assert_eq!(
            desk.fixture.monitor(output)?.active().mode(),
            hidden_mode.toggled()
        );
    }
    assert_eq!(
        desk.fixture.monitor(&desk.hidden)?.active().mode(),
        hidden_mode
    );
    Ok(())
}

#[test]
fn ext_workspace_lists_one_group_per_shown_monitor_in_either_scope() -> anyhow::Result<()> {
    for scope in [WorkspaceScope::PerMonitor, WorkspaceScope::Shared] {
        let desk = desk(scope)?;
        let (groups, workspaces) = workspace_model(&desk.fixture.shell);
        let hidden = desk.id(&desk.hidden)?.raw();

        assert_eq!(groups.len(), 3);
        assert!(groups.iter().all(|group| group.key != hidden));
        assert_eq!(workspaces.len(), 3);
        assert!(workspaces.iter().all(|workspace| workspace.active));
    }
    Ok(())
}

#[test]
fn a_monitor_plugged_into_a_shared_desk_joins_the_workspace_on_screen() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;
    desk.fixture.shell.ensure_workspace(focused, 2);
    desk.fixture.shell.activate_index(focused, 2);

    let late = desk.fixture.plug("DP-4", (1920, 1080));
    desk.refresh();

    let monitor = desk.fixture.monitor(&late)?;
    assert_eq!((monitor.workspaces().len(), monitor.active_index()), (3, 2));
    assert_eq!(desk.shape()?, [(3, 2), (3, 2), (3, 2), (1, 0)]);
    Ok(())
}

fn modes_of(desk: &Desk, output: &Output) -> anyhow::Result<Vec<WorkspaceMode>> {
    Ok(desk.fixture.monitor(output)?.workspace_modes())
}

#[test]
fn a_monitor_joining_a_shared_desk_takes_the_modes_of_the_focused_one() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;
    let changed = desk.fixture.shell.default_mode().toggled();
    desk.fixture.shell.ensure_workspace(focused, 2);
    for index in [0, 2] {
        desk.fixture.shell.activate_index(focused, index);
        desk.fixture.shell.set_workspace_mode_on(focused, changed);
    }

    let late = desk.fixture.plug("DP-4", (1920, 1080));
    desk.refresh();

    let expected = modes_of(&desk, &desk.shown[0])?;
    assert_eq!(expected[0], changed);
    assert_eq!(modes_of(&desk, &late)?, expected);
    assert_eq!(modes_of(&desk, &desk.shown[2])?, expected);
    Ok(())
}

#[test]
fn a_workspace_ensured_on_a_shared_desk_takes_the_focused_monitors_mode() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;
    let last = desk.id(&desk.shown[2])?;
    let changed = desk.fixture.shell.default_mode().toggled();
    desk.fixture.shell.set_workspace_mode_on(focused, changed);
    let late = desk.fixture.plug("DP-4", (1920, 1080));
    desk.refresh();

    desk.fixture.shell.ensure_workspace(last, 1);
    desk.refresh();

    for output in [&desk.shown[0], &desk.shown[2], &late] {
        assert_eq!(modes_of(&desk, output)?[0], changed);
    }
    Ok(())
}

#[test]
fn a_monitor_joining_a_per_monitor_desk_keeps_its_own_modes() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    let focused = desk.focused()?;
    let default = desk.fixture.shell.default_mode();
    desk.fixture
        .shell
        .set_workspace_mode_on(focused, default.toggled());

    let late = desk.fixture.plug("DP-4", (1920, 1080));
    desk.refresh();

    assert_eq!(modes_of(&desk, &late)?, [default]);
    Ok(())
}

#[test]
fn unplugging_the_middle_shared_monitor_keeps_the_others_in_step() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;
    desk.fixture.shell.ensure_workspace(focused, 1);
    desk.fixture.shell.activate_index(focused, 1);
    let middle = desk.shown[1].clone();

    desk.fixture.unplug(&middle);
    desk.refresh();
    let again = desk.fixture.plug("DP-2", (1920, 1080));
    desk.refresh();

    for output in [&desk.shown[0], &again, &desk.shown[2]] {
        let monitor = desk.fixture.monitor(output)?;
        assert_eq!((monitor.workspaces().len(), monitor.active_index()), (2, 1));
    }
    Ok(())
}

fn rule(workspace: Option<u16>, output: Option<&str>) -> config::ResolvedRule {
    config::ResolvedRule {
        workspace,
        output: output.map(str::to_owned),
        ..Default::default()
    }
}

#[test]
fn a_rule_workspace_lands_on_the_rule_output_and_pads_a_shared_desk() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let middle = desk.id(&desk.shown[1])?;

    let location = desk
        .fixture
        .shell
        .rule_location(&rule(Some(2), Some("DP-2")))
        .context("the rule places the window")?;
    desk.refresh();

    assert_eq!(location.output, middle);
    assert_eq!(
        Some(location.workspace),
        desk.fixture.workspace_ids(&desk.shown[1])?.get(2).copied()
    );
    assert_eq!(desk.shape()?, [(3, 0), (3, 0), (3, 0), (1, 0)]);
    Ok(())
}

#[test]
fn a_rule_naming_a_hidden_output_uses_the_focused_monitor() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    let location = desk
        .fixture
        .shell
        .rule_location(&rule(Some(1), Some("HEADLESS-1")))
        .context("the rule places the window")?;
    assert_eq!(location.output, desk.focused()?);
    Ok(())
}

#[test]
fn a_rule_with_only_an_output_takes_its_active_workspace() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    let last = desk.id(&desk.shown[2])?;
    let active = desk.fixture.monitor(&desk.shown[2])?.active().id();

    let location = desk.fixture.shell.rule_location(&rule(None, Some("DP-3")));
    assert_eq!(
        location.map(|location| (location.output, location.workspace)),
        Some((last, active))
    );
    assert!(
        desk.fixture
            .shell
            .rule_location(&rule(None, None))
            .is_none()
    );
    assert!(
        desk.fixture
            .shell
            .rule_location(&rule(Some(40), None))
            .is_none()
    );
    Ok(())
}

#[test]
fn moving_a_per_monitor_workspace_hands_it_to_the_neighbour_active() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    let focused = desk.focused()?;
    desk.fixture.shell.ensure_workspace(focused, 1);
    desk.fixture.shell.activate_index(focused, 1);
    let moving = desk.fixture.monitor(&desk.shown[0])?.active().id();

    let target = desk
        .fixture
        .shell
        .move_workspace_to_output(Direction::Right);
    desk.refresh();

    assert_eq!(target, Some(desk.id(&desk.shown[1])?));
    assert_eq!(desk.fixture.monitor(&desk.shown[1])?.active().id(), moving);
    assert_eq!(desk.shape()?, [(1, 0), (2, 1), (1, 0), (1, 0)]);
    Ok(())
}

#[test]
fn moving_the_last_workspace_leaves_a_fresh_one_behind() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::PerMonitor)?;
    let before = desk.fixture.workspace_ids(&desk.shown[0])?;

    desk.fixture
        .shell
        .move_workspace_to_output(Direction::Right);
    desk.refresh();

    let after = desk.fixture.workspace_ids(&desk.shown[0])?;
    assert_eq!(after.len(), 1);
    assert_ne!(after, before);
    assert!(
        desk.fixture
            .workspace_ids(&desk.shown[1])?
            .contains(&before[0])
    );
    Ok(())
}

#[test]
fn moving_a_shared_workspace_keeps_every_monitor_in_step() -> anyhow::Result<()> {
    let mut desk = desk(WorkspaceScope::Shared)?;
    let focused = desk.focused()?;
    desk.fixture.shell.ensure_workspace(focused, 1);
    desk.fixture.shell.activate_index(focused, 1);

    let target = desk
        .fixture
        .shell
        .move_workspace_to_output(Direction::Right);
    desk.refresh();

    assert_eq!(target, Some(desk.id(&desk.shown[1])?));
    assert_eq!(desk.shape()?, [(2, 1), (2, 1), (2, 1), (1, 0)]);
    Ok(())
}

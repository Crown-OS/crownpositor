//! The keyboard monitor across unplug, replug and an empty desk. With no
//! windows the held window is always none, so these check the fallback and
//! the note that waits; the window side is unit-tested with the rule.

use super::Fixture;

#[test]
fn unplugging_the_keyboard_monitor_hands_the_keyboard_to_the_pointer() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let middle = fixture.plug("DP-2", (1920, 1080));
    let right = fixture.plug("DP-3", (1920, 1080));
    let right_id = fixture.monitor(&right)?.id();
    fixture
        .shell
        .focus_keyboard_on(fixture.monitor(&middle)?.id());
    fixture.shell.focus_at((4000.0, 500.0).into());

    fixture.unplug(&middle);

    assert_eq!(fixture.shell.keyboard, Some(right_id));
    assert!(fixture.shell.keyboard_before_change.is_none());
    assert!(fixture.monitor(&left).is_ok());
    Ok(())
}

#[test]
fn a_replugged_monitor_does_not_take_the_keyboard_from_the_pointer() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let left = fixture.plug("DP-1", (1920, 1080));
    let right = fixture.plug("DP-2", (1920, 1080));
    let left_id = fixture.monitor(&left)?.id();
    fixture.shell.focus_at((100.0, 100.0).into());
    fixture.shell.focus_keyboard_on(left_id);

    fixture.unplug(&right);
    let _right = fixture.plug("DP-2", (1920, 1080));

    assert_eq!(fixture.shell.keyboard, Some(left_id));
    assert!(fixture.shell.keyboard_before_change.is_none());
    Ok(())
}

#[test]
fn the_note_waits_through_an_empty_desk_and_ends_on_the_first_monitor_back() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let only = fixture.plug("eDP-1", (1920, 1080));

    fixture.unplug(&only);
    assert_eq!(fixture.shell.keyboard, None);
    assert!(fixture.shell.keyboard_before_change.is_some());

    let back = fixture.plug("eDP-1", (1920, 1080));
    assert_eq!(fixture.shell.keyboard, Some(fixture.monitor(&back)?.id()));
    assert!(fixture.shell.keyboard_before_change.is_none());
    Ok(())
}

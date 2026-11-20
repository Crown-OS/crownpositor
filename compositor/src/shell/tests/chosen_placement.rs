use anyhow::Context;

use super::{Fixture, Head};
use crate::shell::output_kind::OutputKind;

#[test]
fn a_hotplug_keeps_the_scale_and_position_chosen_for_an_unconfigured_monitor() -> anyhow::Result<()>
{
    let mut fixture = Fixture::new()?;
    let virtual_head = |name| Head::physical(name, (1920, 1080)).of_kind(OutputKind::Virtual);
    fixture.plug_head(virtual_head("VIRTUAL-1"));
    let tablet = fixture.plug_head(virtual_head("VIRTUAL-2"));
    let monitor = fixture.shell.monitor_mut(&tablet).context("plugged")?;
    monitor.choose_scale(2.0);
    monitor.choose_position((0, 1080).into());
    fixture.shell.arrange_outputs();

    let other = fixture.plug_head(virtual_head("VIRTUAL-3"));
    fixture.unplug(&other);
    fixture.shell.apply_output_settings(&fixture.config, None);

    let config = fixture.monitor(&tablet)?.config();
    assert_eq!(config.scale.fractional_scale(), 2.0);
    assert_eq!(config.position, (0, 1080).into());
    Ok(())
}

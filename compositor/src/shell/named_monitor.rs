//! A monitor by a name the user wrote: in a window rule, in `primary_output`,
//! in `WORKSPACE=output:NAME`.

use crate::{
    shell::{Shell, monitor::Monitor},
    utils::head_name,
};

impl Shell {
    /// A monitor by connector name or identity, else by the connector name
    /// it had before it gained a GPU address.
    pub fn monitor_named(&self, name: &str) -> Option<&Monitor> {
        named(|| self.monitors.iter(), name)
    }

    /// [`Self::monitor_named`] among the shown monitors.
    pub fn shown_monitor_named(&self, name: &str) -> Option<&Monitor> {
        named(|| self.shown_monitors(), name)
    }
}

fn named<'a, Monitors>(monitors: impl Fn() -> Monitors, name: &str) -> Option<&'a Monitor>
where
    Monitors: Iterator<Item = &'a Monitor>,
{
    monitors()
        .find(|monitor| {
            monitor.config().name == name || monitor.config().connector.as_str() == name
        })
        .or_else(|| monitors().find(|monitor| head_name::was_named(&monitor.config().name, name)))
}

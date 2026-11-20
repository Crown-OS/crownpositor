//! `WORKSPACE` in a client's environment picks where its windows map:
//! a workspace number on the focused monitor, `headless` for the hidden
//! output, or `output:NAME` for one output by its full name. An agent sets it
//! once and everything it launches stays out of the way; a remote desktop
//! guest's app fills the output streamed to the guest.

use std::str::FromStr;

use smithay::reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface};

use crate::{
    shell::{Location, workspace_scope::MAX_WORKSPACES},
    state::State,
    utils::process_env,
};

pub const VARIABLE: &str = "WORKSPACE";
const OUTPUT_PREFIX: &str = "output:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    /// Zero-based, though the variable counts from one like the shortcuts do.
    Workspace(usize),
    Headless,
    /// The active workspace of the output with this full name, filled by the window.
    Output(String),
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LaunchTargetError {
    #[error("expected a workspace number, `headless` or `output:NAME`, got {0:?}")]
    Unknown(String),
    #[error("workspace {0} is outside 1..={MAX_WORKSPACES}")]
    OutOfRange(usize),
}

impl FromStr for LaunchTarget {
    type Err = LaunchTargetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("headless") {
            return Ok(Self::Headless);
        }
        if let Some(name) = value.strip_prefix(OUTPUT_PREFIX)
            && !name.is_empty()
        {
            return Ok(Self::Output(name.to_owned()));
        }
        let number: usize = value
            .parse()
            .map_err(|_| LaunchTargetError::Unknown(value.to_owned()))?;
        if !(1..=MAX_WORKSPACES).contains(&number) {
            return Err(LaunchTargetError::OutOfRange(number));
        }
        Ok(Self::Workspace(number - 1))
    }
}

impl State {
    /// Where the client owning `surface` was told to put its windows.
    pub fn launch_target(&self, surface: &WlSurface) -> Option<LaunchTarget> {
        let credentials = surface
            .client()?
            .get_credentials(&self.common.display_handle)
            .ok()?;
        let value = process_env::variable(credentials.pid, VARIABLE)?;
        value
            .parse()
            .inspect_err(
                |error| tracing::warn!(%error, pid = credentials.pid, "{VARIABLE} ignored"),
            )
            .ok()
    }

    pub fn launch_location(&mut self, target: &LaunchTarget) -> Option<Location> {
        match target {
            LaunchTarget::Output(name) => {
                let monitor = self.shell.monitor_named(name)?;
                Some(Location {
                    output: monitor.id(),
                    workspace: monitor.active().id(),
                })
            }
            LaunchTarget::Headless => self.headless_location(),
            LaunchTarget::Workspace(index) => {
                let output = self.shell.focused_monitor()?.id();
                Some(Location {
                    output,
                    workspace: self.shell.ensure_workspace(output, *index)?,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_count_from_one() {
        assert_eq!("1".parse(), Ok(LaunchTarget::Workspace(0)));
        assert_eq!("32".parse(), Ok(LaunchTarget::Workspace(31)));
    }

    #[test]
    fn headless_ignores_case() {
        assert_eq!("headless".parse(), Ok(LaunchTarget::Headless));
        assert_eq!("Headless".parse(), Ok(LaunchTarget::Headless));
    }

    #[test]
    fn outputs_are_named_in_full() {
        assert_eq!(
            "output:VIRTUAL-crownlink-ab12cd34".parse(),
            Ok(LaunchTarget::Output(
                "VIRTUAL-crownlink-ab12cd34".to_owned()
            ))
        );
    }

    #[test]
    fn rejects_what_is_not_a_target() {
        assert_eq!(
            "0".parse::<LaunchTarget>(),
            Err(LaunchTargetError::OutOfRange(0))
        );
        assert_eq!(
            "33".parse::<LaunchTarget>(),
            Err(LaunchTargetError::OutOfRange(33))
        );
        for value in ["", "-1", "agent", " 2", "output:"] {
            assert_eq!(
                value.parse::<LaunchTarget>(),
                Err(LaunchTargetError::Unknown(value.to_owned()))
            );
        }
    }
}

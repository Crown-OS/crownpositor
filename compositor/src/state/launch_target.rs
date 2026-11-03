//! `WORKSPACE` in a client's environment picks where its windows map:
//! a workspace number on the focused monitor, or `headless` for the hidden
//! output. An agent sets it once and everything it launches stays out of the
//! way.

use std::str::FromStr;

use smithay::reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface};

use crate::{shell::Location, state::State, utils::process_env};

pub const VARIABLE: &str = "WORKSPACE";
/// Past this a number is a typo, not a workspace anyone means to create.
const MAX_WORKSPACE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchTarget {
    /// Zero-based, though the variable counts from one like the shortcuts do.
    Workspace(usize),
    Headless,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LaunchTargetError {
    #[error("expected a workspace number or `headless`, got {0:?}")]
    Unknown(String),
    #[error("workspace {0} is outside 1..={MAX_WORKSPACE}")]
    OutOfRange(usize),
}

impl FromStr for LaunchTarget {
    type Err = LaunchTargetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("headless") {
            return Ok(Self::Headless);
        }
        let number: usize = value
            .parse()
            .map_err(|_| LaunchTargetError::Unknown(value.to_owned()))?;
        if !(1..=MAX_WORKSPACE).contains(&number) {
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

    pub fn launch_location(&mut self, target: LaunchTarget) -> Option<Location> {
        match target {
            LaunchTarget::Headless => self.headless_location(),
            LaunchTarget::Workspace(index) => {
                let monitor = self.shell.focused_monitor_mut()?;
                Some(Location {
                    output: monitor.id(),
                    workspace: monitor.ensure_workspace(index),
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
    fn rejects_what_is_not_a_target() {
        assert_eq!(
            "0".parse::<LaunchTarget>(),
            Err(LaunchTargetError::OutOfRange(0))
        );
        assert_eq!(
            "33".parse::<LaunchTarget>(),
            Err(LaunchTargetError::OutOfRange(33))
        );
        for value in ["", "-1", "agent", " 2"] {
            assert_eq!(
                value.parse::<LaunchTarget>(),
                Err(LaunchTargetError::Unknown(value.to_owned()))
            );
        }
    }
}

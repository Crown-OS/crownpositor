//! What `crownos_output_info_v1` reports about each monitor, built from the
//! shell and the settings alone so it can be checked without a display.

use config::{Config, OutputSetting};
use crownos_config::schema::ShellRole;
use protocols::crownos_output_info::{Capability, OutputInfo, Shell as ShellSurfaces, State};

use crate::{
    shell::{
        Shell,
        monitor::{Monitor, Presence},
        output_kind::OutputKind,
    },
    utils::id::OutputId,
};

const SHELL_ROLES: [(ShellRole, ShellSurfaces); 3] = [
    (ShellRole::Bar, ShellSurfaces::Bar),
    (ShellRole::Dock, ShellSurfaces::Dock),
    (ShellRole::Notifications, ShellSurfaces::Notifications),
];

/// The shell surfaces an output shows: the user's choice for each role, the
/// default where there is none, and nothing on a hidden output.
pub fn shell_surfaces(
    setting: Option<&OutputSetting>,
    primary: bool,
    presence: Presence,
) -> ShellSurfaces {
    if presence == Presence::Hidden {
        return ShellSurfaces::empty();
    }
    SHELL_ROLES
        .into_iter()
        .filter(|(role, _)| {
            setting.map_or_else(
                || role.default_shown(primary),
                |setting| setting.shows(*role, primary),
            )
        })
        .fold(ShellSurfaces::empty(), |surfaces, (_, bit)| surfaces | bit)
}

impl Shell {
    /// The snapshot for one monitor, given the primary decided once for all
    /// of them.
    pub fn output_info(
        &self,
        monitor: &Monitor,
        primary: Option<OutputId>,
        config: &Config,
    ) -> Option<OutputInfo> {
        let identity = self.identity(monitor.id())?;
        let output = monitor.config();
        let is_primary = primary == Some(monitor.id());
        let setting = config.output_setting(&output.name, identity.as_str());

        let mut state = State::empty();
        if is_primary {
            state |= State::Primary;
        }
        if self.focused == Some(monitor.id()) {
            state |= State::Focused;
        }
        let color = monitor.color();
        let mut capabilities = Capability::empty();
        if color.hdr_capable {
            capabilities |= Capability::Hdr;
        }
        if output.kind == OutputKind::Virtual {
            capabilities |= Capability::Virtual;
        }

        Some(OutputInfo {
            identity: identity.as_str().to_owned(),
            shell: shell_surfaces(setting, is_primary, output.presence),
            state,
            capabilities,
            scanout_depth: color.depth.bits(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(
        bar: Option<bool>,
        dock: Option<bool>,
        notifications: Option<bool>,
    ) -> OutputSetting {
        OutputSetting {
            bar,
            dock,
            notifications,
            ..OutputSetting::default()
        }
    }

    #[test]
    fn a_monitor_with_no_entry_gets_the_bar_and_on_the_primary_the_rest() {
        assert_eq!(
            shell_surfaces(None, true, Presence::Shown),
            ShellSurfaces::Bar | ShellSurfaces::Dock | ShellSurfaces::Notifications
        );
        assert_eq!(
            shell_surfaces(None, false, Presence::Shown),
            ShellSurfaces::Bar
        );
    }

    #[test]
    fn the_users_choice_overrides_each_default() {
        let chosen = setting(Some(false), Some(true), None);
        assert_eq!(
            shell_surfaces(Some(&chosen), false, Presence::Shown),
            ShellSurfaces::Dock
        );
        assert_eq!(
            shell_surfaces(Some(&chosen), true, Presence::Shown),
            ShellSurfaces::Dock | ShellSurfaces::Notifications
        );
    }

    #[test]
    fn an_entry_without_shell_choices_keeps_the_defaults() {
        let unset = setting(None, None, None);
        assert_eq!(
            shell_surfaces(Some(&unset), false, Presence::Shown),
            shell_surfaces(None, false, Presence::Shown)
        );
    }

    #[test]
    fn a_hidden_output_shows_nothing_whatever_the_settings() {
        let everything = setting(Some(true), Some(true), Some(true));
        assert_eq!(
            shell_surfaces(Some(&everything), true, Presence::Hidden),
            ShellSurfaces::empty()
        );
    }
}

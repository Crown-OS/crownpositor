//! Who decides whether an app may expose its UI to AI agents.

use std::ffi::OsStr;

/// Read once at startup. Anything but `allow` denies.
const POLICY_VARIABLE: &str = "CROWNOS_AGENT_ACCESS";

/// How `crownos_agent_access_v1` requests are answered. An interactive
/// prompt becomes another variant here: it parks the request, shows the user
/// who is asking and why, and answers once they choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAccessPolicy {
    GrantAll,
    DenyAll,
}

impl AgentAccessPolicy {
    pub fn from_env() -> Self {
        Self::from_setting(std::env::var_os(POLICY_VARIABLE).as_deref())
    }

    fn from_setting(setting: Option<&OsStr>) -> Self {
        match setting.and_then(OsStr::to_str) {
            Some("allow") => Self::GrantAll,
            _ => Self::DenyAll,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_allow_grants() {
        assert_eq!(
            AgentAccessPolicy::from_setting(Some(OsStr::new("allow"))),
            AgentAccessPolicy::GrantAll
        );
        assert_eq!(
            AgentAccessPolicy::from_setting(None),
            AgentAccessPolicy::DenyAll
        );
        for setting in ["", "1", "yes", "ALLOW", "allow "] {
            assert_eq!(
                AgentAccessPolicy::from_setting(Some(OsStr::new(setting))),
                AgentAccessPolicy::DenyAll
            );
        }
    }
}

//! Who decides whether an app may expose its UI to AI agents: a remembered
//! answer in `agents.ron`, the user through a portal dialog, or an
//! environment override for tests.

use std::{collections::HashMap, ffi::OsStr};

/// Read once at startup. `allow` and `deny` answer everything without asking;
/// anything else leaves the decision to the config and the user.
const POLICY_VARIABLE: &str = "CROWNOS_AGENT_ACCESS";

/// How `crownos_agent_access_v1` requests are answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAccessPolicy {
    GrantAll,
    DenyAll,
    /// Remembered answers first, then the user, then no.
    Ask,
}

impl AgentAccessPolicy {
    pub fn from_env() -> Self {
        Self::from_setting(std::env::var_os(POLICY_VARIABLE).as_deref())
    }

    fn from_setting(setting: Option<&OsStr>) -> Self {
        match setting.and_then(OsStr::to_str) {
            Some("allow") => Self::GrantAll,
            Some("deny") => Self::DenyAll,
            _ => Self::Ask,
        }
    }
}

/// Requests parked while the user is being asked, by the ticket the portal
/// request was made under.
#[derive(Debug)]
pub struct PendingPrompts<Request> {
    next_ticket: u64,
    waiting: HashMap<u64, Request>,
}

impl<Request> Default for PendingPrompts<Request> {
    fn default() -> Self {
        Self {
            next_ticket: 0,
            waiting: HashMap::new(),
        }
    }
}

impl<Request> PendingPrompts<Request> {
    pub fn park(&mut self, request: Request) -> u64 {
        self.next_ticket += 1;
        self.waiting.insert(self.next_ticket, request);
        self.next_ticket
    }

    pub fn take(&mut self, ticket: u64) -> Option<Request> {
        self.waiting.remove(&ticket)
    }

    /// Whether a matching request is already being asked about, so one
    /// dialog serves every request meanwhile.
    pub fn is_asking(&self, matches: impl Fn(&Request) -> bool) -> bool {
        self.waiting.values().any(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_overrides_skip_the_user() {
        assert_eq!(
            AgentAccessPolicy::from_setting(Some(OsStr::new("allow"))),
            AgentAccessPolicy::GrantAll
        );
        assert_eq!(
            AgentAccessPolicy::from_setting(Some(OsStr::new("deny"))),
            AgentAccessPolicy::DenyAll
        );
        assert_eq!(
            AgentAccessPolicy::from_setting(None),
            AgentAccessPolicy::Ask
        );
        for setting in ["", "1", "yes", "ALLOW", "allow "] {
            assert_eq!(
                AgentAccessPolicy::from_setting(Some(OsStr::new(setting))),
                AgentAccessPolicy::Ask
            );
        }
    }
}

//! Where a window rule's `workspace` and `output` put a new window.

use config::ResolvedRule;

use crate::shell::{Location, Shell, monitor::Monitor, workspace_scope::MAX_WORKSPACES};

/// The zero-based workspace index a rule names, unless it is past what
/// anyone means to create.
pub fn rule_workspace_index(workspace: u16) -> Option<usize> {
    let index = usize::from(workspace);
    (index < MAX_WORKSPACES).then_some(index)
}

impl Shell {
    /// The rule's workspace index on the rule's output, or on the focused
    /// monitor when the output is not shown; the active workspace of the
    /// rule's output when it names no workspace. `None` when the rule places
    /// nothing.
    pub fn rule_location(&mut self, rule: &ResolvedRule) -> Option<Location> {
        let named = rule
            .output
            .as_deref()
            .and_then(|name| self.shown_monitor_named(name))
            .map(Monitor::id);
        match rule.workspace {
            Some(workspace) => {
                let index = rule_workspace_index(workspace).or_else(|| {
                    tracing::warn!(workspace, "window rule names a workspace past the limit");
                    None
                })?;
                let output = named.or(self.focused)?;
                Some(Location {
                    output,
                    workspace: self.ensure_workspace(output, index)?,
                })
            }
            None => {
                let monitor = self.monitor_by_id(named?)?;
                Some(Location {
                    output: monitor.id(),
                    workspace: monitor.active().id(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_workspace_is_zero_based_and_bounded() {
        assert_eq!(rule_workspace_index(0), Some(0));
        assert_eq!(rule_workspace_index(31), Some(31));
        assert_eq!(rule_workspace_index(32), None);
        assert_eq!(rule_workspace_index(u16::MAX), None);
    }
}

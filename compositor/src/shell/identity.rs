//! The identity a monitor's settings are filed under.
//!
//! The EDID identity follows a monitor from port to port. Two connected
//! monitors with the same one (twins with no serial, or equal serials) cannot
//! be told apart by it, so both fall back to their connector name and their
//! settings follow the port instead.

use crate::shell::{Shell, monitor::ConnectorId};
use crate::utils::id::OutputId;

#[derive(Debug, Clone, Copy)]
pub struct HeadIdentity<'a> {
    pub connector: &'a str,
    pub edid_identity: &'a str,
}

/// The identity each head goes by, in the same order.
pub fn current_identities<'a>(heads: &[HeadIdentity<'a>]) -> Vec<&'a str> {
    heads
        .iter()
        .map(|head| {
            let twins = heads
                .iter()
                .filter(|other| other.edid_identity == head.edid_identity)
                .count();
            match twins {
                1 => head.edid_identity,
                _ => head.connector,
            }
        })
        .collect()
}

impl Shell {
    /// The identity a monitor goes by now, which changes when its twin is
    /// plugged in or out.
    pub fn identity(&self, output: OutputId) -> Option<&ConnectorId> {
        self.monitor_by_id(output)
            .map(|monitor| &monitor.config().connector)
    }

    /// The identity a monitor about to be added goes by, given the ones
    /// already here.
    pub(super) fn identity_beside_others(
        &self,
        connector: &str,
        edid_identity: &str,
    ) -> ConnectorId {
        let mut heads = self.head_identities();
        heads.push(HeadIdentity {
            connector,
            edid_identity,
        });
        current_identities(&heads).last().map_or_else(
            || ConnectorId::from_identity(edid_identity),
            |identity| ConnectorId::from_identity(identity),
        )
    }

    /// Re-decides every monitor's identity. Returns the monitors whose
    /// identity changed.
    pub(super) fn refresh_identities(&mut self) -> Vec<OutputId> {
        let identities: Vec<ConnectorId> = current_identities(&self.head_identities())
            .into_iter()
            .map(ConnectorId::from_identity)
            .collect();

        let mut changed = Vec::new();
        for (monitor, identity) in self.monitors.iter_mut().zip(identities) {
            if monitor.config().connector != identity {
                monitor.config_mut().connector = identity;
                changed.push(monitor.id());
            }
        }
        changed
    }

    fn head_identities(&self) -> Vec<HeadIdentity<'_>> {
        self.monitors
            .iter()
            .map(|monitor| HeadIdentity {
                connector: &monitor.config().name,
                edid_identity: &monitor.config().edid_identity,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head<'a>(connector: &'a str, edid_identity: &'a str) -> HeadIdentity<'a> {
        HeadIdentity {
            connector,
            edid_identity,
        }
    }

    #[test]
    fn distinct_monitors_keep_their_edid_identity() {
        let heads = [
            head("DP-1", "Dell U2720Q A1"),
            head("DP-2", "Dell U2720Q B2"),
        ];
        assert_eq!(
            current_identities(&heads),
            ["Dell U2720Q A1", "Dell U2720Q B2"]
        );
    }

    #[test]
    fn twins_both_fall_back_to_their_connector() {
        let heads = [
            head("DP-1", "Dell U2720Q A1"),
            head("eDP-1", "eDP-1"),
            head("DP-2", "Dell U2720Q A1"),
        ];
        assert_eq!(current_identities(&heads), ["DP-1", "eDP-1", "DP-2"]);
    }

    #[test]
    fn a_lone_twin_gets_its_edid_identity_back() {
        let heads = [head("DP-2", "Dell U2720Q A1")];
        assert_eq!(current_identities(&heads), ["Dell U2720Q A1"]);
    }
}

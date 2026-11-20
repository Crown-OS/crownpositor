//! The identity a monitor's settings are filed under.
//!
//! The EDID identity follows a monitor from port to port. Two connected
//! monitors with the same one (twins with no serial, or equal serials) cannot
//! be told apart by it, so both fall back to their connector name and their
//! settings follow the port instead.

use crate::shell::{Shell, monitor::ConnectorId, output_kind::OutputKind};
use crate::utils::id::OutputId;

/// A connected head that is switched off. It has no monitor, but a twin of
/// it still has to give up its EDID identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineHead {
    pub connector: String,
    pub edid_identity: String,
    /// The identity it goes by now, decided with the monitors.
    pub identity: String,
    pub kind: OutputKind,
}

impl OfflineHead {
    pub fn new(connector: &str, edid_identity: &str, kind: OutputKind) -> Self {
        Self {
            connector: connector.to_owned(),
            edid_identity: edid_identity.to_owned(),
            identity: edid_identity.to_owned(),
            kind,
        }
    }
}

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

    /// The heads that are connected but switched off, as the backend last
    /// listed them. Returns the monitors whose identity changed.
    pub fn set_offline_heads(&mut self, mut heads: Vec<OfflineHead>) -> Vec<OutputId> {
        heads.retain(|head| self.monitor_by_name(&head.connector).is_none());
        self.offline_heads = heads;
        self.refresh_identities()
    }

    pub fn offline_heads(&self) -> &[OfflineHead] {
        &self.offline_heads
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
        let identities: Vec<String> = current_identities(&self.head_identities())
            .into_iter()
            .map(str::to_owned)
            .collect();
        let (of_monitors, of_offline) = identities.split_at(self.monitors.len());

        for (head, identity) in self.offline_heads.iter_mut().zip(of_offline) {
            head.identity.clone_from(identity);
        }
        let mut changed = Vec::new();
        for (monitor, identity) in self.monitors.iter_mut().zip(of_monitors) {
            if monitor.config().connector.as_str() != identity {
                monitor.config_mut().connector = ConnectorId::from_identity(identity);
                changed.push(monitor.id());
            }
        }
        changed
    }

    /// Every monitor, then every offline head.
    fn head_identities(&self) -> Vec<HeadIdentity<'_>> {
        let monitors = self.monitors.iter().map(|monitor| HeadIdentity {
            connector: &monitor.config().name,
            edid_identity: &monitor.config().edid_identity,
        });
        let offline = self.offline_heads.iter().map(|head| HeadIdentity {
            connector: &head.connector,
            edid_identity: &head.edid_identity,
        });
        monitors.chain(offline).collect()
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

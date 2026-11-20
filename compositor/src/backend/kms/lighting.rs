//! Whether every head an output configuration turns on can have a CRTC,
//! asked before the configuration is accepted.

use std::collections::HashSet;

use smithay::reexports::drm::control::{connector, crtc};

use crate::backend::kms::{
    KmsState,
    crtc_pool::{Assignment, CrtcTopology},
};

/// One head of a GPU, and the CRTC driving it when it is lit.
#[derive(Debug, Clone, Copy)]
pub struct HeadSlot<'a> {
    pub name: &'a str,
    pub connector: connector::Handle,
    pub crtc: Option<crtc::Handle>,
}

/// The connectors to drive once a request is applied, and the CRTCs those
/// already lit keep. `requested` says on or off for a head the request
/// names; a head it does not name stays as it is.
pub fn wanted_after(
    heads: &[HeadSlot],
    requested: impl Fn(&str) -> Option<bool>,
) -> (Vec<connector::Handle>, Assignment) {
    let wanted: Vec<&HeadSlot> = heads
        .iter()
        .filter(|head| requested(head.name).unwrap_or(head.crtc.is_some()))
        .collect();
    let pinned = wanted
        .iter()
        .filter_map(|head| head.crtc.map(|crtc| (head.connector, crtc)))
        .collect();
    (wanted.iter().map(|head| head.connector).collect(), pinned)
}

/// Whether one GPU can drive every head the request leaves on.
pub fn fits(
    topology: &CrtcTopology,
    heads: &[HeadSlot],
    requested: impl Fn(&str) -> Option<bool>,
    leased: &HashSet<crtc::Handle>,
) -> bool {
    let (wanted, pinned) = wanted_after(heads, requested);
    wanted.is_empty() || topology.assign(&wanted, &pinned, leased).is_ok()
}

/// Whether every GPU can drive the heads the request leaves on.
pub fn can_light(kms: &KmsState, requested: impl Fn(&str) -> Option<bool>) -> bool {
    kms.devices.values().all(|device| {
        let heads: Vec<HeadSlot> = device
            .heads
            .values()
            .map(|head| HeadSlot {
                name: &head.name,
                connector: head.connector,
                crtc: head.state.crtc(),
            })
            .collect();
        let connectors: Vec<connector::Handle> = heads.iter().map(|head| head.connector).collect();
        match CrtcTopology::read(&device.drm, &connectors) {
            Ok(topology) => fits(&topology, &heads, &requested, &device.leased_crtcs),
            Err(err) => {
                tracing::warn!(%err, node = %device.node, "cannot read the GPU's CRTCs");
                false
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use smithay::reexports::drm::control::from_u32;

    use super::*;

    fn handle<T: From<std::num::NonZeroU32>>(raw: u32) -> Option<T> {
        from_u32(raw)
    }

    /// Two CRTCs; DP-1 and DP-2 can use either, HDMI-A-1 only the first.
    struct Pool {
        topology: CrtcTopology,
        connectors: [connector::Handle; 3],
        crtcs: [crtc::Handle; 2],
    }

    fn pool() -> Option<Pool> {
        let crtcs = [handle(10)?, handle(11)?];
        let connectors = [handle(1)?, handle(2)?, handle(3)?];
        let possible = HashMap::from([
            (connectors[0], crtcs.to_vec()),
            (connectors[1], crtcs.to_vec()),
            (connectors[2], vec![crtcs[0]]),
        ]);
        Some(Pool {
            topology: CrtcTopology::new(crtcs.to_vec(), possible),
            connectors,
            crtcs,
        })
    }

    fn slots(pool: &Pool, lit: [Option<crtc::Handle>; 3]) -> Vec<HeadSlot<'static>> {
        ["DP-1", "DP-2", "HDMI-A-1"]
            .into_iter()
            .zip(pool.connectors)
            .zip(lit)
            .map(|((name, connector), crtc)| HeadSlot {
                name,
                connector,
                crtc,
            })
            .collect()
    }

    fn request(on: &'static [(&'static str, bool)]) -> impl Fn(&str) -> Option<bool> {
        move |name| {
            on.iter()
                .find(|(head, _)| *head == name)
                .map(|(_, enabled)| *enabled)
        }
    }

    #[test]
    fn a_third_monitor_on_two_crtcs_is_refused() -> Result<(), &'static str> {
        let pool = pool().ok_or("handles")?;
        let heads = slots(&pool, [Some(pool.crtcs[0]), Some(pool.crtcs[1]), None]);
        let none = HashSet::new();
        assert!(!fits(
            &pool.topology,
            &heads,
            request(&[("HDMI-A-1", true)]),
            &none
        ));
        Ok(())
    }

    #[test]
    fn switching_one_off_frees_its_crtc_for_another() -> Result<(), &'static str> {
        let pool = pool().ok_or("handles")?;
        let heads = slots(&pool, [Some(pool.crtcs[0]), Some(pool.crtcs[1]), None]);
        let none = HashSet::new();
        let swap = request(&[("DP-2", false), ("HDMI-A-1", true)]);
        assert!(fits(&pool.topology, &heads, swap, &none));
        Ok(())
    }

    #[test]
    fn a_leased_crtc_is_not_handed_out() -> Result<(), &'static str> {
        let pool = pool().ok_or("handles")?;
        let heads = slots(&pool, [Some(pool.crtcs[1]), None, None]);
        let leased = HashSet::from([pool.crtcs[0]]);
        assert!(!fits(
            &pool.topology,
            &heads,
            request(&[("HDMI-A-1", true)]),
            &leased
        ));
        assert!(fits(&pool.topology, &heads, request(&[]), &leased));
        Ok(())
    }

    #[test]
    fn heads_the_request_leaves_out_keep_their_state_and_crtc() -> Result<(), &'static str> {
        let pool = pool().ok_or("handles")?;
        let heads = slots(&pool, [Some(pool.crtcs[1]), None, None]);
        let (wanted, pinned) = wanted_after(&heads, request(&[("DP-2", true)]));
        assert_eq!(wanted, [pool.connectors[0], pool.connectors[1]]);
        assert_eq!(pinned.get(&pool.connectors[0]), Some(&pool.crtcs[1]));
        Ok(())
    }
}

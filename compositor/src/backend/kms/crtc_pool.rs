//! Deciding which CRTC drives which connector.
//!
//! A GPU has fewer CRTCs than connectors, and each connector can only be
//! driven by some of them — which ones is a per-encoder bitmask. So "can I
//! enable this monitor?" is a bipartite matching problem, and answering it
//! *before* touching the hardware is what makes `zwlr_output_configuration_v1`'s
//! `test` request mean anything.
//!
//! `smithay_drm_extras`' `SimpleCrtcMapper` cannot be reused for this. It
//! assigns greedily across every *connected* connector, reserving a CRTC even
//! for one we have chosen not to light up, so it cannot answer "can I enable B
//! if I disable A?" — which is the only question worth asking.

use std::collections::{HashMap, HashSet};

use smithay::{
    backend::drm::DrmDevice,
    reexports::drm::control::{Device as ControlDevice, ResourceHandles, connector, crtc},
};

#[derive(Debug, thiserror::Error)]
pub enum CrtcError {
    #[error("the GPU has no free CRTC for connector {0:?}")]
    Exhausted(connector::Handle),
    #[error("failed to read the GPU's resources: {0}")]
    Resources(#[source] std::io::Error),
}

/// Which CRTC each requested connector should use.
pub type Assignment = HashMap<connector::Handle, crtc::Handle>;

/// Assigns a CRTC to every connector in `wanted`.
///
/// `pinned` are connectors that already have a live surface; they keep their
/// CRTC wherever the matching allows, because moving one costs a modeset that
/// the user would see as a black flash on a monitor they did not touch.
///
/// `leased` CRTCs belong to a client driving its own display — a VR headset —
/// and are not the compositor's to hand out.
pub fn assign(
    drm: &DrmDevice,
    wanted: &[connector::Handle],
    pinned: &Assignment,
    leased: &HashSet<crtc::Handle>,
) -> Result<Assignment, CrtcError> {
    if wanted.is_empty() {
        return Ok(Assignment::new());
    }

    let resources = drm.resource_handles().map_err(CrtcError::Resources)?;
    let crtcs: Vec<crtc::Handle> = resources
        .crtcs()
        .iter()
        .filter(|crtc| !leased.contains(crtc))
        .copied()
        .collect();
    let crtc_index: HashMap<crtc::Handle, usize> = crtcs
        .iter()
        .enumerate()
        .map(|(index, handle)| (*handle, index))
        .collect();

    let candidates: Vec<Vec<usize>> = wanted
        .iter()
        .map(|connector| {
            possible_crtcs(drm, &resources, *connector)
                .into_iter()
                .filter_map(|handle| crtc_index.get(&handle).copied())
                .collect()
        })
        .collect();

    let seeds: Vec<Option<usize>> = wanted
        .iter()
        .map(|connector| pinned.get(connector).and_then(|crtc| crtc_index.get(crtc)).copied())
        .collect();

    let matched = match_connectors(&candidates, &seeds, crtcs.len())
        .map_err(|index| CrtcError::Exhausted(wanted[index]))?;

    Ok(wanted
        .iter()
        .zip(matched)
        .map(|(connector, crtc)| (*connector, crtcs[crtc]))
        .collect())
}

/// The CRTCs the kernel says can drive this connector.
///
/// The union over the connector's encoders. `CrtcListFilter` is opaque, so the
/// masks cannot be OR-ed together and each encoder is resolved separately.
fn possible_crtcs(
    drm: &DrmDevice,
    resources: &ResourceHandles,
    connector: connector::Handle,
) -> Vec<crtc::Handle> {
    let Ok(info) = drm.get_connector(connector, false) else {
        return Vec::new();
    };

    let mut allowed: Vec<crtc::Handle> = Vec::new();
    for encoder in info.encoders() {
        let Ok(encoder) = drm.get_encoder(*encoder) else {
            continue;
        };
        for crtc in resources.filter_crtcs(encoder.possible_crtcs()) {
            if !allowed.contains(&crtc) {
                allowed.push(crtc);
            }
        }
    }
    allowed
}

/// Maximum bipartite matching by augmenting paths (Kuhn's algorithm).
///
/// Exact rather than greedy: the connector count is single digits, so the
/// difference is free, and a greedy pass reports "no CRTC" for arrangements
/// that are perfectly assignable — it would make `test` refuse configurations
/// `apply` could have carried out.
///
/// `seeds` are preferred starting assignments. They are not guaranteed: an
/// augmenting path may displace one, which is correct, since keeping it would
/// mean failing a configuration that is achievable.
///
/// `Err(index)` names a connector that could not be given a CRTC.
fn match_connectors(
    candidates: &[Vec<usize>],
    seeds: &[Option<usize>],
    crtcs: usize,
) -> Result<Vec<usize>, usize> {
    // Which connector owns each CRTC.
    let mut owner: Vec<Option<usize>> = vec![None; crtcs];

    for (connector, seed) in seeds.iter().enumerate() {
        if let Some(crtc) = seed
            && candidates[connector].contains(crtc)
            && owner[*crtc].is_none()
        {
            owner[*crtc] = Some(connector);
        }
    }

    for connector in 0..candidates.len() {
        if owner.iter().any(|owner| *owner == Some(connector)) {
            continue;
        }
        let mut visited = vec![false; crtcs];
        if !augment(connector, candidates, &mut owner, &mut visited) {
            return Err(connector);
        }
    }

    (0..candidates.len())
        .map(|connector| {
            owner
                .iter()
                .position(|owner| *owner == Some(connector))
                .ok_or(connector)
        })
        .collect()
}

fn augment(
    connector: usize,
    candidates: &[Vec<usize>],
    owner: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for &crtc in &candidates[connector] {
        if visited[crtc] {
            continue;
        }
        visited[crtc] = true;

        let free = owner[crtc].is_none();
        if free || augment(owner[crtc].unwrap_or(connector), candidates, owner, visited) {
            owner[crtc] = Some(connector);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matched(candidates: &[Vec<usize>], crtcs: usize) -> Result<Vec<usize>, usize> {
        let seeds = vec![None; candidates.len()];
        match_connectors(candidates, &seeds, crtcs)
    }

    #[test]
    fn every_connector_gets_its_own_crtc() {
        let assignment = matched(&[vec![0, 1], vec![0, 1]], 2).expect("two fit in two");
        assert_ne!(assignment[0], assignment[1]);
    }

    #[test]
    fn more_connectors_than_crtcs_is_refused() {
        assert_eq!(matched(&[vec![0], vec![0]], 1), Err(1));
    }

    #[test]
    fn a_connector_no_crtc_can_drive_is_refused() {
        assert_eq!(matched(&[vec![]], 2), Err(0));
    }

    /// The case greedy assignment gets wrong: the first connector must give up
    /// CRTC 0 so the second, which can only use CRTC 0, can have it.
    #[test]
    fn an_augmenting_path_reassigns_rather_than_failing() {
        let assignment = matched(&[vec![0, 1], vec![0]], 2).expect("this is assignable");
        assert_eq!(assignment[1], 0);
        assert_eq!(assignment[0], 1);
    }

    #[test]
    fn a_chain_of_reassignments_is_followed() {
        // 0 can use {0,1}, 1 can use {1,2}, 2 can use {2} — everything has to
        // shift along by one.
        let assignment =
            matched(&[vec![0, 1], vec![1, 2], vec![2]], 3).expect("this is assignable");
        assert_eq!(assignment, vec![0, 1, 2]);
    }

    #[test]
    fn a_pinned_connector_keeps_its_crtc_when_it_can() {
        let candidates = [vec![0, 1], vec![0, 1]];
        let assignment =
            match_connectors(&candidates, &[Some(1), None], 2).expect("two fit in two");
        assert_eq!(assignment[0], 1, "the pin should have been honoured");
        assert_eq!(assignment[1], 0);
    }

    #[test]
    fn a_pin_is_given_up_rather_than_failing_the_configuration() {
        // The pin would starve the second connector, which can only use CRTC 0.
        let candidates = [vec![0, 1], vec![0]];
        let assignment =
            match_connectors(&candidates, &[Some(0), None], 2).expect("this is assignable");
        assert_eq!(assignment[1], 0);
        assert_eq!(assignment[0], 1);
    }

    #[test]
    fn an_impossible_pin_is_ignored() {
        // CRTC 5 is not among the connector's candidates.
        let candidates = [vec![0]];
        assert_eq!(match_connectors(&candidates, &[Some(1)], 2), Ok(vec![0]));
    }

    #[test]
    fn nothing_wanted_assigns_nothing() {
        assert_eq!(matched(&[], 4), Ok(Vec::new()));
    }
}

/// A CRTC that could drive `connector` and is not already in use.
///
/// Used when granting a DRM lease: the client needs a CRTC of its own, and it
/// must be one the compositor is not about to hand to a monitor.
pub fn free_for(
    drm: &DrmDevice,
    connector: connector::Handle,
    taken: &HashSet<crtc::Handle>,
) -> Option<crtc::Handle> {
    let resources = drm.resource_handles().ok()?;
    possible_crtcs(drm, &resources, connector)
        .into_iter()
        .find(|crtc| !taken.contains(crtc))
}

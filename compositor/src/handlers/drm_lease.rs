//! `wp_drm_lease_v1`: handing a connector to a client that drives it itself.
//!
//! A VR headset is not a desktop display — the kernel marks its connector
//! `non-desktop`, and the compositor must never light it up. Instead the
//! headset's runtime asks for a *lease*: the connector, a CRTC and a plane,
//! for it to drive directly with its own timings.
//!
//! The resources in a lease leave the compositor's pool for as long as the
//! lease lives, which is why `leased_crtcs` exists — handing the same CRTC to
//! a monitor would fight the client for the hardware.

use smithay::{
    backend::drm::DrmNode,
    delegate_drm_lease,
    wayland::drm_lease::{
        DrmLease, DrmLeaseBuilder, DrmLeaseHandler, DrmLeaseRequest, DrmLeaseState, LeaseRejected,
    },
};

use crate::{backend::kms::crtc_pool, state::State};

impl DrmLeaseHandler for State {
    fn drm_lease_state(&mut self, node: DrmNode) -> &mut DrmLeaseState {
        self.backend
            .kms()
            .and_then(|kms| kms.devices.get_mut(&node))
            .and_then(|device| device.lease_state.as_mut())
            .expect("a lease request can only arrive for a device that has a lease global")
    }

    fn lease_request(
        &mut self,
        node: DrmNode,
        request: DrmLeaseRequest,
    ) -> Result<DrmLeaseBuilder, LeaseRejected> {
        let Some(device) = self
            .backend
            .kms()
            .and_then(|kms| kms.devices.get_mut(&node))
        else {
            return Err(LeaseRejected::default());
        };

        let mut builder = DrmLeaseBuilder::new(&device.drm);
        // Everything the compositor is using, plus whatever earlier leases
        // took — a CRTC can only drive one thing at a time.
        let mut taken: std::collections::HashSet<_> = device
            .surfaces
            .keys()
            .copied()
            .chain(device.leased_crtcs.iter().copied())
            .collect();

        for connector in request.connectors {
            // A client must not be able to lease the panel the desktop is on.
            if device
                .heads
                .get(&connector)
                .is_some_and(|head| head.state.is_enabled())
            {
                tracing::warn!("refusing to lease a connector the desktop is using");
                return Err(LeaseRejected::default());
            }

            let Some(crtc) = crtc_pool::free_for(&device.drm, connector, &taken) else {
                tracing::warn!("refusing a lease: no CRTC is free for the requested connector");
                return Err(LeaseRejected::default());
            };

            let planes = device.drm.planes(&crtc).map_err(LeaseRejected::with_cause)?;
            // A CRTC always has at least one primary plane; a driver that
            // disagrees is one this compositor cannot drive anyway.
            let Some(primary) = planes.primary.first().map(|plane| plane.handle) else {
                tracing::warn!("refusing a lease: the CRTC has no primary plane");
                return Err(LeaseRejected::default());
            };
            let Some(claim) = device.drm.claim_plane(primary, crtc) else {
                tracing::warn!("refusing a lease: the primary plane is already claimed");
                return Err(LeaseRejected::default());
            };

            builder.add_connector(connector);
            builder.add_crtc(crtc);
            builder.add_plane(primary, claim);
            taken.insert(crtc);
        }

        Ok(builder)
    }

    fn new_active_lease(&mut self, node: DrmNode, lease: DrmLease) {
        let Some(device) = self
            .backend
            .kms()
            .and_then(|kms| kms.devices.get_mut(&node))
        else {
            return;
        };

        device.leased_crtcs.extend(lease.crtcs().copied());
        tracing::info!(id = lease.id(), %node, "granted a DRM lease");
        // Held so that dropping it revokes the lease; the client losing its
        // connection has to give the hardware back.
        device.active_leases.insert(lease.id(), lease);
    }

    fn lease_destroyed(&mut self, node: DrmNode, lease_id: u32) {
        let Some(device) = self
            .backend
            .kms()
            .and_then(|kms| kms.devices.get_mut(&node))
        else {
            return;
        };

        if let Some(lease) = device.active_leases.remove(&lease_id) {
            for crtc in lease.crtcs() {
                device.leased_crtcs.remove(crtc);
            }
        }
        tracing::info!(id = lease_id, %node, "a DRM lease ended");
    }
}

delegate_drm_lease!(State);

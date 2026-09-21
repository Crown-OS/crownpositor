//! Pushing an accepted output configuration onto the hardware.
//!
//! Everything here runs *after* `State::reconfigure_outputs` has validated the
//! request, so a failure at this point is a surprise from the driver rather
//! than a bad client — which is why each one is logged and reported rather
//! than silently swallowed.

use anyhow::Context as _;
use smithay::{
    backend::drm::{DrmDevice, VrrSupport},
    output::{Mode, Output},
    reexports::drm::control::{Device as ControlDevice, connector},
};

use protocols::gamma_control::GammaRamps;

use crate::backend::{
    frame_clock::FrameClock,
    kms::{KmsState, surface::Surface},
};

/// Whether a mode can actually be set on this output, without setting it.
///
/// The cached mode list on a `Monitor` can be stale — a KVM switch or a
/// re-read EDID changes it — so the answer comes from the connector as it is
/// right now.
pub fn supports_mode(kms: &KmsState, output: &Output, mode: Mode) -> bool {
    let Some((device, connector)) = connector_for(kms, output) else {
        // No surface means no hardware to disagree; validation upstream has
        // already checked the mode against what was advertised.
        return true;
    };

    device
        .get_connector(connector, false)
        .map(|info| {
            info.modes()
                .iter()
                .any(|candidate| Mode::from(*candidate) == mode)
        })
        .unwrap_or(false)
}

/// What the connector can do about adaptive sync.
pub fn vrr_support(kms: &KmsState, output: &Output) -> VrrSupport {
    kms.devices
        .values()
        .flat_map(|device| device.surfaces.values())
        .find(|surface| surface.output == *output)
        .and_then(|surface| surface.compositor.vrr_supported(surface_connector(surface)?).ok())
        .unwrap_or(VrrSupport::NotSupported)
}

/// Changes an output's mode, for real.
///
/// Order matters and is the whole point of this function. `DrmCompositor` was
/// built with `OutputModeSource::Auto`, so it takes its render size from the
/// `Output`; the caller must therefore have already updated the `Output`
/// before the next frame, or the commit is rejected on a size mismatch. The
/// swapchain is resized here and the modeset itself lands on the next
/// `queue_frame`.
pub fn set_mode(kms: &mut KmsState, output: &Output, mode: Mode) -> anyhow::Result<()> {
    let drm_mode = find_drm_mode(kms, output, mode)
        .with_context(|| format!("output {} has no mode {mode:?}", output.name()))?;

    let Some(surface) = surface_mut(kms, output) else {
        anyhow::bail!("output {} has no surface to reconfigure", output.name());
    };

    if surface.compositor.pending_mode() == drm_mode {
        return Ok(());
    }

    surface
        .compositor
        .use_mode(drm_mode)
        .with_context(|| format!("the driver refused mode {mode:?}"))?;

    // Buffer ages are meaningless across a swapchain resize, so the next frame
    // has to be a full repaint.
    surface.compositor.reset_buffers();
    surface.frame_clock = FrameClock::new(refresh_interval(mode), surface.compositor.vrr_enabled());
    surface.redraw_state = std::mem::take(&mut surface.redraw_state).queue();

    Ok(())
}

/// Whether adaptive sync is on for this output right now.
pub fn vrr_enabled(kms: &KmsState, output: &Output) -> bool {
    kms.devices
        .values()
        .flat_map(|device| device.surfaces.values())
        .find(|surface| surface.output == *output)
        .is_some_and(|surface| surface.compositor.vrr_enabled())
}

/// Turns adaptive sync on or off.
pub fn set_vrr(kms: &mut KmsState, output: &Output, enabled: bool) -> anyhow::Result<()> {
    let Some(surface) = surface_mut(kms, output) else {
        anyhow::bail!("output {} has no surface to reconfigure", output.name());
    };

    if surface.compositor.vrr_enabled() == enabled {
        return Ok(());
    }

    surface
        .compositor
        .use_vrr(enabled)
        .with_context(|| format!("the driver refused adaptive sync on {}", output.name()))?;

    // Driven from what the compositor *did*, not what was asked: the frame
    // clock's presentation anchor was measured under the old pacing regime and
    // resetting it against a request the driver ignored would mispredict every
    // frame.
    surface
        .frame_clock
        .set_vrr(surface.compositor.vrr_enabled());
    surface.redraw_state = std::mem::take(&mut surface.redraw_state).queue();

    Ok(())
}

fn refresh_interval(mode: Mode) -> Option<std::time::Duration> {
    (mode.refresh > 0).then(|| {
        std::time::Duration::from_nanos(1_000_000_000_000 / mode.refresh as u64)
    })
}

fn find_drm_mode(
    kms: &KmsState,
    output: &Output,
    mode: Mode,
) -> Option<smithay::reexports::drm::control::Mode> {
    let (device, connector) = connector_for(kms, output)?;
    let info = device.get_connector(connector, false).ok()?;

    info.modes()
        .iter()
        .find(|candidate| Mode::from(**candidate) == mode)
        .copied()
        // Refresh rates round differently between the protocol and the
        // kernel, so fall back to an exact size match at the closest rate.
        .or_else(|| {
            info.modes()
                .iter()
                .filter(|candidate| {
                    let candidate = Mode::from(**candidate);
                    candidate.size == mode.size
                })
                .min_by_key(|candidate| (Mode::from(**candidate).refresh - mode.refresh).abs())
                .copied()
        })
}

fn surface_mut<'a>(kms: &'a mut KmsState, output: &Output) -> Option<&'a mut Surface> {
    kms.devices
        .values_mut()
        .flat_map(|device| device.surfaces.values_mut())
        .find(|surface| surface.output == *output)
}

/// The DRM device and connector behind an output.
fn connector_for<'a>(
    kms: &'a KmsState,
    output: &Output,
) -> Option<(&'a DrmDevice, connector::Handle)> {
    for device in kms.devices.values() {
        for surface in device.surfaces.values() {
            if surface.output == *output
                && let Some(connector) = surface_connector(surface)
            {
                return Some((&device.drm, connector));
            }
        }
    }
    None
}

/// A surface drives exactly one connector in this compositor — cloned
/// connectors would need their own `DrmCompositor`.
fn surface_connector(surface: &Surface) -> Option<connector::Handle> {
    surface
        .compositor
        .surface()
        .current_connectors()
        .into_iter()
        .next()
}

/// How many entries one gamma ramp has, or `None` when this CRTC has no
/// programmable gamma.
pub fn gamma_size(kms: &KmsState, output: &Output) -> Option<u32> {
    let (device, crtc) = crtc_for(kms, output)?;
    let size = device.get_crtc(crtc).ok()?.gamma_length();
    (size > 0).then_some(size)
}

/// Programs the CRTC's gamma ramps, or restores the identity ramp.
///
/// The legacy `SETGAMMA` ioctl rather than the `GAMMA_LUT` property: on an
/// atomic driver the kernel translates it into the property itself, and doing
/// it this way keeps the ramp out of the surface's own atomic commits, so a
/// night-light change costs no modeset and no dropped frame.
pub fn set_gamma(kms: &KmsState, output: &Output, ramps: Option<&GammaRamps>) -> bool {
    let Some((device, crtc)) = crtc_for(kms, output) else {
        return false;
    };
    let Some(size) = gamma_size(kms, output) else {
        return false;
    };

    let identity;
    let ramps = match ramps {
        Some(ramps) if ramps.len() == size as usize => ramps,
        Some(ramps) => {
            tracing::warn!(
                output = output.name(),
                given = ramps.len(),
                expected = size,
                "gamma ramp is the wrong length"
            );
            return false;
        }
        None => {
            identity = identity_ramps(size);
            &identity
        }
    };

    match device.set_gamma(crtc, &ramps.red, &ramps.green, &ramps.blue) {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(%err, output = output.name(), "the driver refused the gamma ramp");
            false
        }
    }
}

/// A ramp that changes nothing, spread evenly across the 16-bit range.
fn identity_ramps(size: u32) -> GammaRamps {
    let last = size.saturating_sub(1).max(1) as u64;
    let channel: Vec<u16> = (0..size)
        .map(|index| (u64::from(index) * u64::from(u16::MAX) / last) as u16)
        .collect();

    GammaRamps {
        red: channel.clone(),
        green: channel.clone(),
        blue: channel,
    }
}

fn crtc_for<'a>(
    kms: &'a KmsState,
    output: &Output,
) -> Option<(&'a DrmDevice, smithay::reexports::drm::control::crtc::Handle)> {
    kms.devices.values().find_map(|device| {
        device
            .surfaces
            .iter()
            .find(|(_, surface)| surface.output == *output)
            .map(|(crtc, _)| (&device.drm, *crtc))
    })
}

/// Whether an output's panel is powered.
pub fn output_power(kms: &KmsState, output: &Output) -> Option<bool> {
    kms.devices
        .values()
        .flat_map(|device| device.surfaces.values())
        .find(|surface| surface.output == *output)
        .map(|surface| surface.powered)
}

/// Blanks an output's panel, or wakes it.
///
/// Powering down disables the CRTC through `DrmCompositor::clear`, which is
/// what actually tells the panel to sleep — rendering black would leave the
/// backlight on. Waking up is just a redraw: the next commit re-enables the
/// CRTC on its own.
pub fn set_output_power(kms: &mut KmsState, output: &Output, on: bool) -> bool {
    let Some(surface) = surface_mut(kms, output) else {
        return false;
    };
    if surface.powered == on {
        return true;
    }

    if on {
        surface.powered = true;
        // Buffer ages are meaningless after the CRTC was off.
        surface.compositor.reset_buffers();
        surface.redraw_state = std::mem::take(&mut surface.redraw_state).queue();
        return true;
    }

    if let Err(err) = surface.compositor.clear() {
        tracing::warn!(%err, output = output.name(), "failed to power the output down");
        return false;
    }
    surface.powered = false;
    true
}

//! Every connector the GPU has, whether or not it is lit.
//!
//! A `Monitor` in the shell is an output that is *on* — it owns workspaces and
//! a `wl_output` global, and turning it off means tearing both down. But a
//! monitor that is plugged in and switched off still has to appear in
//! `zwlr_output_management_v1` with its modes and its identity, or the user
//! has no way to switch it back on.
//!
//! So the backend keeps its own list. This is the only place that knows a
//! connector exists before the shell does.

use std::ops::RangeInclusive;

use smithay::{
    backend::drm::DrmDevice,
    output::Subpixel,
    reexports::drm::control::{Device as ControlDevice, Mode as DrmMode, ModeTypeFlags, connector},
};

use crate::{
    backend::kms::props,
    shell::monitor::ConnectorId,
    utils::edid::EdidInfo,
};

/// Whether this connector is currently driving an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadState {
    Enabled { crtc: smithay::reexports::drm::control::crtc::Handle },
    Disabled,
}

impl HeadState {
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled { .. })
    }

    pub fn crtc(&self) -> Option<smithay::reexports::drm::control::crtc::Handle> {
        match self {
            Self::Enabled { crtc } => Some(*crtc),
            Self::Disabled => None,
        }
    }
}

/// One connector, as the backend knows it.
#[derive(Debug, Clone)]
pub struct Head {
    /// `"DP-1"`, built the same way the shell names the output.
    pub name: String,
    pub connector: connector::Handle,
    /// Survives a replug; what a config entry may be keyed on.
    pub identity: ConnectorId,
    pub edid: Option<EdidInfo>,
    pub modes: Vec<DrmMode>,
    /// Index into [`Self::modes`].
    pub preferred: Option<usize>,
    /// Millimetres.
    pub physical_mm: (u32, u32),
    pub subpixel: Subpixel,
    pub vrr_capable: bool,
    /// Current value and the range the driver accepts.
    pub max_bpc: Option<(u64, RangeInclusive<u64>)>,
    pub state: HeadState,
}

impl Head {
    /// Reads everything about a connector that does not need a CRTC.
    ///
    /// Returns `None` for a connector with no modes — one that is plugged in
    /// but cannot display anything is not something the user can configure.
    pub fn probe(drm: &DrmDevice, connector: connector::Handle) -> Option<Self> {
        let info = drm.get_connector(connector, true).ok()?;
        if info.modes().is_empty() {
            return None;
        }

        let name = format!(
            "{}-{}",
            info.interface().as_str(),
            info.interface_id()
        );
        let edid = props::edid(drm, connector);

        let physical_mm = edid
            .as_ref()
            .and_then(|edid| edid.physical_size_mm)
            .map(|(width, height)| (width as u32, height as u32))
            .or_else(|| info.size())
            .unwrap_or((0, 0));

        let identity = ConnectorId::new(
            &name,
            edid.as_ref().map_or("Unknown", |edid| edid.make.as_str()),
            edid.as_ref().map_or("Unknown", |edid| edid.model.as_str()),
            edid.as_ref().and_then(|edid| edid.serial.as_deref()),
        );

        let modes: Vec<DrmMode> = info.modes().to_vec();
        let preferred = modes
            .iter()
            .position(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED));

        let vrr_capable = props::vrr_capable(drm, connector);
        let max_bpc = props::max_bpc(drm, connector);
        tracing::debug!(
            output = name,
            identity = identity.as_str(),
            modes = modes.len(),
            vrr = vrr_capable,
            ?max_bpc,
            "probed a connector"
        );

        Some(Self {
            name,
            connector,
            identity,
            edid,
            modes,
            preferred,
            physical_mm,
            subpixel: info.subpixel().into(),
            vrr_capable,
            max_bpc,
            state: HeadState::Disabled,
        })
    }

    /// Whether the driver would accept a `max bpc` of `bpc`.
    ///
    /// Actually *setting* it needs a connector property in the atomic commit,
    /// which smithay does not expose yet — but the range is readable now, so a
    /// configured value that could never work is worth saying so about rather
    /// than silently ignoring.
    pub fn accepts_max_bpc(&self, bpc: u8) -> bool {
        self.max_bpc
            .as_ref()
            .is_some_and(|(_, range)| range.contains(&u64::from(bpc)))
    }

    /// The mode to light this connector up with, given what the config asks.
    ///
    /// Falls back to the preferred mode, then to the first one — a connector
    /// with no modes never becomes a `Head` in the first place.
    pub fn mode_for(&self, wanted: Option<&str>) -> Option<DrmMode> {
        if let Some(text) = wanted
            && let Some(mode) = self.find_mode(text)
        {
            return Some(mode);
        }
        if wanted.is_some() {
            tracing::warn!(output = self.name, mode = wanted, "no such mode; using the default");
        }

        self.preferred
            .and_then(|index| self.modes.get(index))
            .or_else(|| self.modes.first())
            .copied()
    }

    /// Matches `"2560x1440@144.000"` or `"2560x1440"` against the mode list.
    fn find_mode(&self, text: &str) -> Option<DrmMode> {
        let (size, refresh) = match text.split_once('@') {
            Some((size, refresh)) => (size, refresh.parse::<f64>().ok()),
            None => (text, None),
        };
        let (width, height) = size.split_once('x')?;
        let (width, height) = (width.parse::<u16>().ok()?, height.parse::<u16>().ok()?);

        let matching = self
            .modes
            .iter()
            .filter(|mode| mode.size() == (width, height));

        match refresh {
            // Compared in millihertz against the same arithmetic smithay uses,
            // so a mode named from a head list round-trips exactly.
            Some(wanted) => {
                let wanted = (wanted * 1000.0).round() as i64;
                matching
                    .min_by_key(|mode| {
                        (smithay::output::Mode::from(**mode).refresh as i64 - wanted).abs()
                    })
                    .copied()
            }
            None => matching
                .max_by_key(|mode| smithay::output::Mode::from(**mode).refresh)
                .copied(),
        }
    }
}

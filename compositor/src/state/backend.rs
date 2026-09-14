use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf, drm::VrrSupport, renderer::ImportDma, session::Session as _,
    },
    output::{Mode, Output},
};

use protocols::gamma_control::GammaRamps;

use crate::backend::{
    kms::{KmsState, head::Head, reconfigure},
    winit::WinitState,
};

/// A connected monitor that is switched off, as the protocol needs to see it.
///
/// A plain data copy rather than a borrow of the backend's `Head`, because the
/// caller building head snapshots already holds `&State`.
pub struct DisabledHead {
    pub name: String,
    pub identity: String,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub physical_mm: (i32, i32),
    pub modes: Vec<Mode>,
    pub preferred: Option<usize>,
    pub vrr_capable: bool,
}

impl DisabledHead {
    fn from_head(head: &Head) -> Self {
        Self {
            name: head.name.clone(),
            identity: head.identity.as_str().to_owned(),
            make: head.edid.as_ref().map(|edid| edid.make.clone()),
            model: head.edid.as_ref().map(|edid| edid.model.clone()),
            serial: head.edid.as_ref().and_then(|edid| edid.serial.clone()),
            physical_mm: (head.physical_mm.0 as i32, head.physical_mm.1 as i32),
            modes: head.modes.iter().copied().map(Mode::from).collect(),
            preferred: head.preferred,
            vrr_capable: head.vrr_capable,
        }
    }
}

/// Which backend is driving the session.
///
/// A closed set rather than `Box<dyn Backend>`: the render path needs each
/// backend's concrete renderer and damage tracker, and boxing would only push
/// that behind a downcast. Adding one means adding a variant and an arm to the
/// two methods below — nothing outside this file and the backend's own module
/// should ever name a variant.
pub enum BackendState {
    Unset,
    Winit(Box<WinitState>),
    Kms(Box<KmsState>),
}

impl BackendState {
    pub fn try_new() -> anyhow::Result<Self> {
        Ok(Self::Unset)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Unset => "none",
            Self::Winit(_) => "winit",
            Self::Kms(_) => "kms",
        }
    }

    /// Schedules a frame for an output.
    ///
    /// `None` means "every output this backend drives", which for a
    /// single-output backend collapses to its one output.
    pub fn queue_redraw(&mut self, output: Option<&Output>) {
        match self {
            Self::Unset => {}
            Self::Winit(winit) => {
                if output.is_none_or(|output| *output == winit.output) {
                    winit.backend.window().request_redraw();
                }
            }
            Self::Kms(kms) => kms.queue_redraw(output),
        }
    }

    /// Whether this backend's renderer can import a dmabuf.
    ///
    /// Lives here so `handlers/dmabuf.rs` does not have to know which renderer
    /// is in play.
    pub fn can_import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self {
            Self::Unset => false,
            Self::Winit(winit) => winit.backend.renderer().import_dmabuf(dmabuf, None).is_ok(),
            Self::Kms(kms) => kms.can_import_dmabuf(dmabuf),
        }
    }

    /// Hands the seat to another virtual terminal.
    ///
    /// Only the KMS backend owns a session to switch; nested backends have a
    /// host compositor sitting between them and the VT, so this is a no-op
    /// there rather than an error.
    pub fn switch_vt(&mut self, vt: i32) {
        match self {
            Self::Unset | Self::Winit(_) => {
                tracing::debug!(vt, backend = self.name(), "no session to switch VTs on");
            }
            Self::Kms(kms) => {
                if let Err(err) = kms.session.change_vt(vt) {
                    tracing::error!(%err, vt, "failed to switch VT");
                }
            }
        }
    }

    /// Whether this output could be set to `mode` right now.
    ///
    /// Asked before a configuration is accepted, so `test` can refuse a mode
    /// the connector has stopped advertising — a KVM switch or a re-read EDID
    /// changes the list under a client that was holding a stale one.
    pub fn supports_mode(&self, output: &Output, mode: Mode) -> bool {
        match self {
            // A nested window has exactly the one mode the host gave it.
            Self::Unset | Self::Winit(_) => output.current_mode() == Some(mode),
            Self::Kms(kms) => reconfigure::supports_mode(kms, output, mode),
        }
    }

    /// What the connector can do about adaptive sync.
    pub fn vrr_support(&self, output: &Output) -> VrrSupport {
        match self {
            Self::Unset | Self::Winit(_) => VrrSupport::NotSupported,
            Self::Kms(kms) => reconfigure::vrr_support(kms, output),
        }
    }

    /// Every head the backend knows that is *not* currently an output.
    ///
    /// A monitor the user switched off has no `Monitor` in the shell — that is
    /// the whole point — so this is the only way the protocol can still offer
    /// it, and the only way it can be switched back on.
    pub fn disabled_heads(&self) -> Vec<DisabledHead> {
        match self {
            Self::Unset | Self::Winit(_) => Vec::new(),
            Self::Kms(kms) => kms
                .devices
                .values()
                .flat_map(|device| device.heads.values())
                .filter(|head| !head.state.is_enabled())
                .map(DisabledHead::from_head)
                .collect(),
        }
    }

    /// Turns a head on or off, by the name the protocol knows it as.
    ///
    /// `Ok(false)` means there is no such head, which is not an error: a
    /// configuration naming an output that has just been unplugged is a race,
    /// not a bug.
    pub fn set_head_enabled(
        state: &mut crate::state::State,
        name: &str,
        enabled: bool,
    ) -> anyhow::Result<bool> {
        let Some(kms) = state.backend.kms() else {
            anyhow::bail!("this backend cannot enable or disable outputs");
        };

        let found = kms.devices.iter().find_map(|(node, device)| {
            device
                .heads
                .values()
                .find(|head| head.name == name || head.identity.as_str() == name)
                .map(|head| (*node, head.connector, head.state.is_enabled()))
        });
        let Some((node, connector, already)) = found else {
            return Ok(false);
        };
        if already == enabled {
            return Ok(true);
        }

        if enabled {
            crate::backend::kms::enable_configured_head(state, node, connector)?;
        } else {
            crate::backend::kms::surface::disable_head(state, node, connector);
        }
        Ok(true)
    }

    /// How many entries one gamma ramp has for this output.
    pub fn gamma_size(&self, output: &Output) -> Option<u32> {
        match self {
            // A nested window has no CRTC, so nothing to program.
            Self::Unset | Self::Winit(_) => None,
            Self::Kms(kms) => reconfigure::gamma_size(kms, output),
        }
    }

    /// Programs an output's gamma ramps, or restores the identity ramp.
    pub fn set_gamma(&self, output: &Output, ramps: Option<&GammaRamps>) -> bool {
        match self {
            Self::Unset | Self::Winit(_) => false,
            Self::Kms(kms) => reconfigure::set_gamma(kms, output, ramps),
        }
    }

    /// Whether an output's panel is powered, or `None` if it has no panel.
    pub fn output_power(&self, output: &Output) -> Option<bool> {
        match self {
            Self::Unset | Self::Winit(_) => None,
            Self::Kms(kms) => reconfigure::output_power(kms, output),
        }
    }

    /// Blanks an output's panel, or wakes it.
    pub fn set_output_power(&mut self, output: &Output, on: bool) -> bool {
        match self {
            Self::Unset | Self::Winit(_) => false,
            Self::Kms(kms) => reconfigure::set_output_power(kms, output, on),
        }
    }

    /// Sets an output's mode on the hardware.
    ///
    /// The caller must already have updated the `Output` itself: the KMS
    /// backend's `DrmCompositor` reads its render size from there, so a frame
    /// rendered between the two would be committed at the wrong size.
    pub fn set_output_mode(&mut self, output: &Output, mode: Mode) -> anyhow::Result<()> {
        match self {
            Self::Unset | Self::Winit(_) => Ok(()),
            Self::Kms(kms) => reconfigure::set_mode(kms, output, mode),
        }
    }

    pub fn set_output_vrr(&mut self, output: &Output, enabled: bool) -> anyhow::Result<()> {
        match self {
            Self::Unset | Self::Winit(_) => {
                anyhow::ensure!(!enabled, "this backend has no adaptive sync");
                Ok(())
            }
            Self::Kms(kms) => reconfigure::set_vrr(kms, output, enabled),
        }
    }

    /// Only `backend/winit.rs` should call this. Everything else goes through the
    /// methods above, so a new backend does not ripple into unrelated code.
    pub(crate) fn winit(&mut self) -> Option<&mut WinitState> {
        match self {
            Self::Winit(winit) => Some(winit),
            _ => None,
        }
    }

    /// Only `backend/kms/` should call this, for the same reason as `winit`.
    pub(crate) fn kms(&mut self) -> Option<&mut KmsState> {
        match self {
            Self::Kms(kms) => Some(kms),
            _ => None,
        }
    }
}

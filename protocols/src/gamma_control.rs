//! Server-side `zwlr_gamma_control_unstable_v1`.
//!
//! A client hands over three lookup tables — one per channel — and the
//! compositor programs them into the CRTC's gamma ramp. It is how
//! `gammastep`, `wlsunset` and the desktop's own night-light shift the screen
//! warm after sunset, and it costs nothing per frame because the hardware does
//! the lookup during scanout.
//!
//! Exactly one client may control an output's gamma at a time. A second one
//! taking over is normal rather than an error, so the first is told with
//! `failed` and its object goes inert.

use std::{
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
};

use wayland_protocols_wlr::gamma_control::v1::server::{
    zwlr_gamma_control_manager_v1::{self, ZwlrGammaControlManagerV1},
    zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::GlobalId, protocol::wl_output::WlOutput,
};

/// One lookup table per channel, each `gamma_size` entries long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GammaRamps {
    pub red: Vec<u16>,
    pub green: Vec<u16>,
    pub blue: Vec<u16>,
}

impl GammaRamps {
    pub fn len(&self) -> usize {
        self.red.len()
    }

    pub fn is_empty(&self) -> bool {
        self.red.is_empty()
    }
}

pub trait GammaControlHandler {
    fn gamma_control_state(&mut self) -> &mut GammaControlState;

    /// How many entries one ramp has, or `None` when the output has no
    /// programmable gamma — a virtual output, or a driver without it.
    fn gamma_size(&mut self, output: &WlOutput) -> Option<u32>;

    /// Programs the ramps, or restores the identity ramp when `None`.
    ///
    /// `false` means the driver refused, which becomes `failed` on the wire.
    fn set_gamma(&mut self, output: &WlOutput, ramps: Option<GammaRamps>) -> bool;
}

/// Which clients may see the manager global.
pub struct GammaControlGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// User data of a `zwlr_gamma_control_v1`.
#[derive(Debug)]
pub struct GammaControlData {
    output: WlOutput,
    /// Set when the object has been told `failed`; every later request on it
    /// is then ignored rather than being a protocol error.
    inert: Arc<Mutex<bool>>,
}

pub struct GammaControlState {
    global: GlobalId,
    /// At most one live control per output.
    controls: Vec<(WlOutput, ZwlrGammaControlV1)>,
}

impl GammaControlState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlGlobalData>
            + Dispatch<ZwlrGammaControlManagerV1, ()>
            + Dispatch<ZwlrGammaControlV1, GammaControlData>
            + GammaControlHandler
            + 'static,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        Self {
            global: display.create_global::<D, ZwlrGammaControlManagerV1, _>(
                1,
                GammaControlGlobalData {
                    filter: Box::new(filter),
                },
            ),
            controls: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Tells whoever holds this output's gamma that they no longer do.
    pub fn revoke(&mut self, output: &WlOutput) {
        self.revoke_matching(|held| held == output);
    }

    /// The same, for every control whose output `matches`.
    ///
    /// The compositor calls this when it takes gamma back for itself — its own
    /// night-light setting outranks a client's. It has to be a predicate
    /// rather than a `WlOutput`: one monitor is a `wl_output` *per client*,
    /// and the compositor is taking the monitor, not one client's view of it.
    pub fn revoke_matching(&mut self, mut matches: impl FnMut(&WlOutput) -> bool) {
        self.controls.retain(|(output, control)| {
            if !matches(output) {
                return true;
            }
            fail(control);
            false
        });
    }

    fn claim(&mut self, output: &WlOutput, control: ZwlrGammaControlV1) {
        self.revoke(output);
        self.controls.push((output.clone(), control));
    }

    fn release(&mut self, control: &ZwlrGammaControlV1) -> Option<WlOutput> {
        let index = self
            .controls
            .iter()
            .position(|(_, existing)| existing == control)?;
        Some(self.controls.remove(index).0)
    }
}

/// Marks a control dead and tells the client.
fn fail(control: &ZwlrGammaControlV1) {
    if let Some(data) = control.data::<GammaControlData>() {
        *data.inert.lock().unwrap_or_else(|err| err.into_inner()) = true;
    }
    control.failed();
}

impl<D> GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlGlobalData, D> for GammaControlState
where
    D: GlobalDispatch<ZwlrGammaControlManagerV1, GammaControlGlobalData>
        + Dispatch<ZwlrGammaControlManagerV1, ()>
        + Dispatch<ZwlrGammaControlV1, GammaControlData>
        + GammaControlHandler
        + 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrGammaControlManagerV1>,
        _global_data: &GammaControlGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &GammaControlGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrGammaControlManagerV1, (), D> for GammaControlState
where
    D: Dispatch<ZwlrGammaControlManagerV1, ()>
        + Dispatch<ZwlrGammaControlV1, GammaControlData>
        + GammaControlHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrGammaControlManagerV1,
        request: zwlr_gamma_control_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } = request else {
            return;
        };

        let inert = Arc::new(Mutex::new(false));
        let control = data_init.init(
            id,
            GammaControlData {
                output: output.clone(),
                inert: Arc::clone(&inert),
            },
        );

        let Some(size) = state.gamma_size(&output) else {
            // An output whose gamma cannot be programmed: the client is told
            // now rather than after it has built its tables.
            *inert.lock().unwrap_or_else(|err| err.into_inner()) = true;
            control.failed();
            return;
        };

        control.gamma_size(size);
        state.gamma_control_state().claim(&output, control);
    }
}

impl<D> Dispatch<ZwlrGammaControlV1, GammaControlData, D> for GammaControlState
where
    D: Dispatch<ZwlrGammaControlV1, GammaControlData> + GammaControlHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrGammaControlV1,
        request: zwlr_gamma_control_v1::Request,
        data: &GammaControlData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let zwlr_gamma_control_v1::Request::SetGamma { fd } = request else {
            return;
        };
        if *data.inert.lock().unwrap_or_else(|err| err.into_inner()) {
            return;
        }

        let Some(size) = state.gamma_size(&data.output) else {
            fail(resource);
            return;
        };

        let ramps = match read_ramps(&fd, size as usize) {
            Ok(ramps) => ramps,
            Err(RampError::Malformed) => {
                resource.post_error(
                    zwlr_gamma_control_v1::Error::InvalidGamma,
                    "the gamma tables are not the size this output advertised",
                );
                return;
            }
            // An unreadable fd is the client's problem but not a protocol
            // violation, so the object dies rather than the connection.
            Err(RampError::Unreadable) => {
                fail(resource);
                state.gamma_control_state().release(resource);
                return;
            }
        };

        if !state.set_gamma(&data.output, Some(ramps)) {
            fail(resource);
            state.gamma_control_state().release(resource);
        }
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        resource: &ZwlrGammaControlV1,
        _data: &GammaControlData,
    ) {
        // Whoever was shifting the screen warm has gone; leaving their last
        // table programmed would tint the display until the next reboot.
        if let Some(output) = state.gamma_control_state().release(resource) {
            state.set_gamma(&output, None);
        }
    }
}

enum RampError {
    /// The file is not the length the advertised size requires.
    Malformed,
    Unreadable,
}

/// Reads `3 * size` native-endian `u16`s out of the client's file.
///
/// Read rather than mapped: a client can `ftruncate` the file after handing it
/// over, and a mapping would turn that into a `SIGBUS` in the compositor.
/// The descriptor is duplicated so the `File` has something of its own to
/// close — reading through the client's own descriptor would move its offset.
fn read_ramps(fd: &OwnedFd, size: usize) -> Result<GammaRamps, RampError> {
    use std::os::unix::fs::FileExt;

    let expected = size
        .checked_mul(3)
        .and_then(|entries| entries.checked_mul(2))
        .ok_or(RampError::Malformed)?;

    let file = std::fs::File::from(fd.try_clone().map_err(|_| RampError::Unreadable)?);
    let length = file.metadata().map_err(|_| RampError::Unreadable)?.len();
    if length < expected as u64 {
        return Err(RampError::Malformed);
    }

    let mut bytes = vec![0u8; expected];
    file.read_exact_at(&mut bytes, 0)
        .map_err(|_| RampError::Unreadable)?;

    let mut channels = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
        .collect::<Vec<u16>>();
    let blue = channels.split_off(size * 2);
    let green = channels.split_off(size);

    Ok(GammaRamps {
        red: channels,
        green,
        blue,
    })
}

/// Wires `$ty` up as the dispatch target for this protocol.
#[macro_export]
macro_rules! delegate_gamma_control {
    ($ty:ty) => {
        ::wayland_server::delegate_global_dispatch!($ty: [
            $crate::gamma_control::reexports::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1:
                $crate::gamma_control::GammaControlGlobalData
        ] => $crate::gamma_control::GammaControlState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::gamma_control::reexports::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1: ()
        ] => $crate::gamma_control::GammaControlState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::gamma_control::reexports::zwlr_gamma_control_v1::ZwlrGammaControlV1:
                $crate::gamma_control::GammaControlData
        ] => $crate::gamma_control::GammaControlState);
    };
}

#[doc(hidden)]
pub mod reexports {
    pub use wayland_protocols_wlr::gamma_control::v1::server::*;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    fn ramp_file(entries: &[u16]) -> OwnedFd {
        let mut file = tempfile().expect("a temp file");
        for value in entries {
            file.write_all(&value.to_ne_bytes()).expect("write");
        }
        file.flush().expect("flush");
        file.rewind().expect("rewind");
        OwnedFd::from(file)
    }

    fn tempfile() -> std::io::Result<std::fs::File> {
        let path = std::env::temp_dir().join(format!(
            "crownpositor-gamma-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        // Unlinked immediately: the descriptor keeps it alive and nothing is
        // left behind if the test fails.
        let _ = std::fs::remove_file(&path);
        Ok(file)
    }

    #[test]
    fn the_three_channels_are_split_in_order() {
        let fd = ramp_file(&[1, 2, 10, 20, 100, 200]);
        let ramps = read_ramps(&fd, 2).map_err(|_| ()).expect("a valid ramp file");

        assert_eq!(ramps.red, vec![1, 2]);
        assert_eq!(ramps.green, vec![10, 20]);
        assert_eq!(ramps.blue, vec![100, 200]);
        assert_eq!(ramps.len(), 2);
    }

    #[test]
    fn a_short_file_is_malformed_rather_than_read_past_its_end() {
        let fd = ramp_file(&[1, 2, 3]);
        assert!(matches!(read_ramps(&fd, 2), Err(RampError::Malformed)));
    }

    #[test]
    fn a_longer_file_is_accepted_and_the_tail_ignored() {
        let fd = ramp_file(&[1, 2, 10, 20, 100, 200, 999, 999]);
        let ramps = read_ramps(&fd, 2).map_err(|_| ()).expect("a valid ramp file");
        assert_eq!(ramps.blue, vec![100, 200]);
    }

    #[test]
    fn an_absurd_size_does_not_overflow() {
        let fd = ramp_file(&[1]);
        assert!(matches!(
            read_ramps(&fd, usize::MAX / 2),
            Err(RampError::Malformed)
        ));
    }
}

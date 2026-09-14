//! Server-side `zwlr_output_power_management_unstable_v1`.
//!
//! Blanking a screen without forgetting it exists. An idle daemon uses this to
//! power the panel down after a timeout; the output keeps its `wl_output`, its
//! workspaces and its windows, and comes back exactly as it was.
//!
//! That is what makes it different from disabling an output through
//! `zwlr_output_management_v1`: there, the monitor genuinely leaves the
//! desktop and its windows move elsewhere.
//!
//! One client controls an output's power at a time. A second taking over is
//! normal — the first is told with `failed`.

use std::sync::{Arc, Mutex};

use wayland_protocols_wlr::output_power_management::v1::server::{
    zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
    zwlr_output_power_v1::{self, Mode, ZwlrOutputPowerV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::GlobalId, protocol::wl_output::WlOutput,
};

pub trait OutputPowerHandler {
    fn output_power_state(&mut self) -> &mut OutputPowerState;

    /// Whether the output is powered, or `None` if it cannot be controlled.
    fn output_power(&mut self, output: &WlOutput) -> Option<bool>;

    /// Powers an output up or down. `false` means the driver refused.
    fn set_output_power(&mut self, output: &WlOutput, on: bool) -> bool;
}

pub struct OutputPowerGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

#[derive(Debug)]
pub struct OutputPowerData {
    output: WlOutput,
    inert: Arc<Mutex<bool>>,
}

pub struct OutputPowerState {
    global: GlobalId,
    controls: Vec<(WlOutput, ZwlrOutputPowerV1)>,
}

impl OutputPowerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerGlobalData>
            + Dispatch<ZwlrOutputPowerManagerV1, ()>
            + Dispatch<ZwlrOutputPowerV1, OutputPowerData>
            + OutputPowerHandler
            + 'static,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        Self {
            global: display.create_global::<D, ZwlrOutputPowerManagerV1, _>(
                1,
                OutputPowerGlobalData {
                    filter: Box::new(filter),
                },
            ),
            controls: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Tells every client watching this output what its power state is now.
    ///
    /// Called when the compositor changes it for its own reasons — a lid
    /// closing, a VT switch — so a client's view never goes stale.
    pub fn notify(&self, output: &WlOutput, on: bool) {
        for (owned, control) in &self.controls {
            if owned == output {
                control.mode(if on { Mode::On } else { Mode::Off });
            }
        }
    }

    /// Ends a client's control of this output.
    pub fn revoke(&mut self, output: &WlOutput) {
        self.controls.retain(|(owned, control)| {
            if owned != output {
                return true;
            }
            fail(control);
            false
        });
    }

    fn claim(&mut self, output: &WlOutput, control: ZwlrOutputPowerV1) {
        self.revoke(output);
        self.controls.push((output.clone(), control));
    }

    fn release(&mut self, control: &ZwlrOutputPowerV1) {
        self.controls
            .retain(|(_, existing)| existing != control);
    }
}

fn fail(control: &ZwlrOutputPowerV1) {
    if let Some(data) = control.data::<OutputPowerData>() {
        *data.inert.lock().unwrap_or_else(|err| err.into_inner()) = true;
    }
    control.failed();
}

impl<D> GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerGlobalData, D> for OutputPowerState
where
    D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerGlobalData>
        + Dispatch<ZwlrOutputPowerManagerV1, ()>
        + Dispatch<ZwlrOutputPowerV1, OutputPowerData>
        + OutputPowerHandler
        + 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        _global_data: &OutputPowerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &OutputPowerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<ZwlrOutputPowerManagerV1, (), D> for OutputPowerState
where
    D: Dispatch<ZwlrOutputPowerManagerV1, ()>
        + Dispatch<ZwlrOutputPowerV1, OutputPowerData>
        + OutputPowerHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrOutputPowerManagerV1,
        request: zwlr_output_power_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } = request else {
            return;
        };

        let inert = Arc::new(Mutex::new(false));
        let control = data_init.init(
            id,
            OutputPowerData {
                output: output.clone(),
                inert: Arc::clone(&inert),
            },
        );

        let Some(on) = state.output_power(&output) else {
            *inert.lock().unwrap_or_else(|err| err.into_inner()) = true;
            control.failed();
            return;
        };

        control.mode(if on { Mode::On } else { Mode::Off });
        state.output_power_state().claim(&output, control);
    }
}

impl<D> Dispatch<ZwlrOutputPowerV1, OutputPowerData, D> for OutputPowerState
where
    D: Dispatch<ZwlrOutputPowerV1, OutputPowerData> + OutputPowerHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrOutputPowerV1,
        request: zwlr_output_power_v1::Request,
        data: &OutputPowerData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let zwlr_output_power_v1::Request::SetMode { mode } = request else {
            return;
        };
        if *data.inert.lock().unwrap_or_else(|err| err.into_inner()) {
            return;
        }

        let on = match mode {
            WEnum::Value(Mode::On) => true,
            WEnum::Value(Mode::Off) => false,
            _ => {
                resource.post_error(
                    zwlr_output_power_v1::Error::InvalidMode,
                    "power mode is outside the enum",
                );
                return;
            }
        };

        if state.set_output_power(&data.output, on) {
            // Echoed rather than assumed: the protocol wants the event even
            // when the state did not change, so a client that asked for `off`
            // twice is not left waiting.
            resource.mode(if on { Mode::On } else { Mode::Off });
        } else {
            fail(resource);
            state.output_power_state().release(resource);
        }
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        resource: &ZwlrOutputPowerV1,
        data: &OutputPowerData,
    ) {
        state.output_power_state().release(resource);
        // An idle daemon that crashed while the screen was off would otherwise
        // leave the user with a black display and no way to wake it.
        if state.output_power(&data.output) == Some(false) {
            state.set_output_power(&data.output, true);
        }
    }
}

/// Wires `$ty` up as the dispatch target for this protocol.
#[macro_export]
macro_rules! delegate_output_power {
    ($ty:ty) => {
        ::wayland_server::delegate_global_dispatch!($ty: [
            $crate::output_power::reexports::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1:
                $crate::output_power::OutputPowerGlobalData
        ] => $crate::output_power::OutputPowerState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::output_power::reexports::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1: ()
        ] => $crate::output_power::OutputPowerState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::output_power::reexports::zwlr_output_power_v1::ZwlrOutputPowerV1:
                $crate::output_power::OutputPowerData
        ] => $crate::output_power::OutputPowerState);
    };
}

#[doc(hidden)]
pub mod reexports {
    pub use wayland_protocols_wlr::output_power_management::v1::server::*;
}

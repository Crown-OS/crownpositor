//! Server-side `crownos_input_v1`.
//!
//! Injectors are pure bookkeeping here: events are buffered until `frame` and
//! handed to the compositor as one [`InjectedFrame`], which feeds them through
//! its normal input path. Captures are handles the compositor sends events
//! through once its pointer crosses an armed edge; the geometry for deciding
//! that lives in [`edge`] so it can be tested without a seat.

mod dispatch;
pub mod edge;
mod event;

use std::sync::Mutex;

use crownos_protocols::input::v1::server::{
    crownos_input_capture_v1::CrownosInputCaptureV1,
    crownos_input_manager_v1::CrownosInputManagerV1,
};
use smithay::utils::{Logical, Point};
use wayland_server::{
    Client, DisplayHandle, Resource,
    backend::{GlobalId, ObjectId},
    protocol::wl_output::WlOutput,
};

pub use crownos_protocols::input::v1::server::crownos_input_capture_v1::{
    Axis as CaptureAxis, ButtonState as CaptureButtonState, Edge, KeyState as CaptureKeyState,
};
pub use dispatch::InputDispatch;
pub use edge::{EdgeCrossing, crossed_edge, output_point_to_layout};
pub use event::{Axis, ButtonState, HeldInput, InjectedEvent, InjectedFrame, KeyState, TouchError};

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`InputState`].
pub trait InputHandler {
    fn input_protocol_state(&mut self) -> &mut InputState;

    /// Applies one frame of injected events, in order.
    fn inject_input(&mut self, frame: InjectedFrame);

    /// A capture's armed edges changed; zero disarms it.
    fn arm_input_capture(&mut self, capture: &InputCapture, edges: Edge);

    /// The client gave input back, asking for the pointer at `position` in
    /// `output`'s logical space.
    fn release_input_capture(
        &mut self,
        capture: &InputCapture,
        output: &WlOutput,
        position: Point<f64, Logical>,
    );

    /// The capture object is gone; if it held the seat, input returns.
    fn input_capture_destroyed(&mut self, capture: &InputCapture);
}

/// Global data: which clients may see the manager.
pub struct InputGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Delegate type for the `crownos_input_manager_v1` global.
pub struct InputState {
    global: GlobalId,
}

impl InputState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: InputDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosInputManagerV1, _>(
            VERSION,
            InputGlobalData {
                filter: Box::new(filter),
            },
        );
        Self { global }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

#[derive(Debug, Default)]
struct InjectorInner {
    pending: Vec<InjectedEvent>,
    held: HeldInput,
}

/// User data of a `crownos_input_injector_v1`.
#[derive(Debug, Default)]
pub struct InjectorData {
    inner: Mutex<InjectorInner>,
}

impl InjectorData {
    fn with<T>(&self, change: impl FnOnce(&mut InjectorInner) -> T) -> T {
        change(
            &mut self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// User data of a `crownos_input_capture_v1`.
#[derive(Debug, Default)]
pub struct CaptureData;

/// The compositor's handle on a capture, through which captured input goes out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputCapture(CrownosInputCaptureV1);

impl InputCapture {
    pub fn id(&self) -> ObjectId {
        self.0.id()
    }

    pub fn is_alive(&self) -> bool {
        self.0.is_alive()
    }

    pub fn client(&self) -> Option<Client> {
        self.0.client()
    }

    pub fn entered(&self, output: &WlOutput, edge: Edge, position: f64) {
        self.0.entered(output, edge, position);
    }

    pub fn motion(&self, delta: Point<f64, Logical>) {
        self.0.motion(delta.x, delta.y);
    }

    pub fn button(&self, button: u32, state: CaptureButtonState) {
        self.0.button(button, state);
    }

    pub fn axis(&self, axis: CaptureAxis, value: f64, value120: i32) {
        self.0.axis(axis, value, value120);
    }

    pub fn key(&self, key: u32, state: CaptureKeyState) {
        self.0.key(key, state);
    }

    pub fn modifiers(&self, depressed: u32, latched: u32, locked: u32, group: u32) {
        self.0.modifiers(depressed, latched, locked, group);
    }

    pub fn frame(&self, time_usec: u64) {
        self.0.frame((time_usec >> 32) as u32, time_usec as u32);
    }

    pub fn released(&self) {
        if self.0.is_alive() {
            self.0.released();
        }
    }
}

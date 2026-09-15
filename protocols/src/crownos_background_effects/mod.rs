//! Server-side `crownos_background_effects`.
//!
//! Lets a client ask the compositor to render the material its surface sits on:
//! a blurred, tinted and saturated backdrop under a parametric shape, a
//! refractive rim along that shape's edge, a corner radius clipping the surface
//! itself, and a drop shadow cast underneath it.
//!
//! Shapes are parametric rather than pixel regions on purpose. A `wl_region` is
//! a set of integer rectangles and cannot describe a rounded corner without
//! thousands of one-pixel steps; the shapes here go straight into a signed
//! distance field in the renderer's fragment shader, so they are antialiased at
//! the output's real resolution at no per-rectangle cost.
//!
//! Like every other module in this crate, this one is only bookkeeping: it
//! validates requests, applies the double-buffered state at commit, and leaves
//! the result on the surface for the renderer to read at element-build time.
//! Nothing here touches a renderer or a backend.

mod color;
mod dispatch;
mod effects;
mod shape;

pub use color::Argb;
pub use effects::{Blur, CommittedEffects, Effects, Shadow, surface_effects};
pub use shape::{MAX_PRIMITIVES, Primitives, RoundedRect, ShapeData};

pub use crownos_protocols::background_effects::v1::server::crownos_background_effect_manager_v1::Capability;

use crownos_protocols::background_effects::v1::server::{
    crownos_background_effect_manager_v1::CrownosBackgroundEffectManagerV1,
    crownos_shape_manager_v1::CrownosShapeManagerV1,
};
use wayland_server::{DisplayHandle, Resource, Weak, backend::GlobalId};

use crate::crownos_background_effects::dispatch::BackgroundEffectsData;

/// What the compositor has to provide for this protocol to be delegated to
/// [`BackgroundEffectsState`].
pub trait BackgroundEffectsHandler {
    fn background_effects_state(&mut self) -> &mut BackgroundEffectsState;
}

/// User data of both globals and of every manager bound from them.
///
/// Empty because the capabilities are read from [`BackgroundEffectsState`] on
/// each bind rather than snapshotted here, which is what lets them change
/// afterwards.
#[derive(Debug, Clone, Copy)]
pub struct GlobalData;

/// Delegate type for the two globals this protocol defines.
#[derive(Debug)]
pub struct BackgroundEffectsState {
    shapes: GlobalId,
    effects: GlobalId,
    capabilities: Capability,
    /// Every effect manager a client currently holds, so a capability change
    /// can be announced — the protocol promises the event "every time the set
    /// changes", not just on bind.
    managers: Vec<Weak<CrownosBackgroundEffectManagerV1>>,
}

impl BackgroundEffectsState {
    /// Registers both globals. `capabilities` is what gets advertised on bind —
    /// narrow it to what the renderer will actually draw, and clients know not
    /// to ask for the rest.
    pub fn new<D: BackgroundEffectsData>(
        display: &DisplayHandle,
        capabilities: Capability,
    ) -> Self {
        Self {
            shapes: display.create_global::<D, CrownosShapeManagerV1, _>(1, GlobalData),
            effects: display.create_global::<D, CrownosBackgroundEffectManagerV1, _>(1, GlobalData),
            capabilities,
            managers: Vec::new(),
        }
    }

    pub fn shape_global(&self) -> GlobalId {
        self.shapes.clone()
    }

    pub fn effect_global(&self) -> GlobalId {
        self.effects.clone()
    }

    pub fn capabilities(&self) -> Capability {
        self.capabilities
    }

    /// Announces a new capability set to every bound manager.
    ///
    /// "When a capability goes away the corresponding effect stops being
    /// rendered even if it was set before" — so the renderer is free to stop
    /// drawing the moment this is called; committed state stays committed and
    /// takes effect again if the capability comes back.
    pub fn set_capabilities(&mut self, capabilities: Capability) {
        // Pruned regardless of whether anything changed, so a client that binds
        // and disconnects in a loop cannot grow this list without bound.
        self.managers.retain(|manager| manager.upgrade().is_ok());
        if self.capabilities == capabilities {
            return;
        }

        self.capabilities = capabilities;
        for manager in &self.managers {
            if let Ok(manager) = manager.upgrade() {
                manager.capabilities(capabilities);
            }
        }
    }

    fn remember(&mut self, manager: &CrownosBackgroundEffectManagerV1) {
        self.managers.retain(|manager| manager.upgrade().is_ok());
        self.managers.push(manager.downgrade());
    }
}

/// User data of a [`CrownosBackgroundEffectSurfaceV1`] object.
///
/// [`CrownosBackgroundEffectSurfaceV1`]: crownos_protocols::background_effects::v1::server::crownos_background_effect_surface_v1::CrownosBackgroundEffectSurfaceV1
#[derive(Debug)]
pub struct EffectSurfaceData {
    /// Holds the surface weakly: the object goes inert when its surface dies,
    /// and a strong handle here would keep the surface alive instead.
    ///
    /// `None` marks an object that lost the race for its surface's one effect
    /// slot. Such an object exists only so the protocol error has something live
    /// to be posted on, and must never touch the surface — least of all release
    /// the slot its rightful owner is holding.
    surface: Option<Weak<wayland_server::protocol::wl_surface::WlSurface>>,
}

impl EffectSurfaceData {
    fn new(surface: &wayland_server::protocol::wl_surface::WlSurface) -> Self {
        Self {
            surface: Some(surface.downgrade()),
        }
    }

    fn duplicate() -> Self {
        Self { surface: None }
    }

    fn wl_surface(&self) -> Option<wayland_server::protocol::wl_surface::WlSurface> {
        self.surface.as_ref()?.upgrade().ok()
    }
}

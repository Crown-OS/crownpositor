//! Server-side `wp-tearing-control-v1` (staging).
//!
//! A client says whether its frames may be shown the moment they are ready,
//! tearing, instead of waiting for vblank. The hint is double-buffered on the
//! surface; what the compositor does with it — only for a fullscreen window,
//! only when the user allows it — is policy that lives with the compositor.

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::wayland::compositor::{Cacheable, with_states};
use wayland_protocols::wp::tearing_control::v1::server::{
    wp_tearing_control_manager_v1::{self, WpTearingControlManagerV1},
    wp_tearing_control_v1::{self, WpTearingControlV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum, Weak,
    backend::{ClientId, GlobalId},
    protocol::wl_surface::WlSurface,
};

pub use wp_tearing_control_v1::PresentationHint;

/// The hint a client committed. Vsync until it says otherwise, and again once
/// its control object is gone.
#[derive(Debug, Clone, Copy)]
pub struct TearingHintCachedState {
    pub hint: PresentationHint,
}

impl Default for TearingHintCachedState {
    fn default() -> Self {
        Self {
            hint: PresentationHint::Vsync,
        }
    }
}

impl Cacheable for TearingHintCachedState {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        *self
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

/// Whether the client asked for its frames to be presented without waiting
/// for vblank.
pub fn wants_async_presentation(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states.cached_state.has::<TearingHintCachedState>()
            && matches!(
                states
                    .cached_state
                    .get::<TearingHintCachedState>()
                    .current()
                    .hint,
                PresentationHint::Async
            )
    })
}

/// Enforces "one tearing-control object per surface". In the surface's
/// `data_map`, which outlives any one object.
#[derive(Debug, Default)]
struct ControlSlot(AtomicBool);

/// User data of a [`WpTearingControlV1`]. `None` marks the object a client
/// created in violation of the one-per-surface rule, which exists only to
/// carry the protocol error and must never touch the surface.
#[derive(Debug)]
pub struct TearingControlData(Option<Weak<WlSurface>>);

impl TearingControlData {
    fn surface(&self) -> Option<WlSurface> {
        self.0.as_ref()?.upgrade().ok()
    }
}

#[derive(Debug)]
pub struct TearingControlState {
    global: GlobalId,
}

impl TearingControlState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<WpTearingControlManagerV1, ()>
            + Dispatch<WpTearingControlManagerV1, ()>
            + Dispatch<WpTearingControlV1, TearingControlData>
            + 'static,
    {
        Self {
            global: display.create_global::<D, WpTearingControlManagerV1, _>(1, ()),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

impl<D> GlobalDispatch<WpTearingControlManagerV1, (), D> for TearingControlState
where
    D: GlobalDispatch<WpTearingControlManagerV1, ()>
        + Dispatch<WpTearingControlManagerV1, ()>
        + Dispatch<WpTearingControlV1, TearingControlData>
        + 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WpTearingControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D> Dispatch<WpTearingControlManagerV1, (), D> for TearingControlState
where
    D: Dispatch<WpTearingControlManagerV1, ()>
        + Dispatch<WpTearingControlV1, TearingControlData>
        + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        manager: &WpTearingControlManagerV1,
        request: wp_tearing_control_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let wp_tearing_control_manager_v1::Request::GetTearingControl { id, surface } = request
        else {
            return;
        };
        let taken = with_states(&surface, |states| {
            states
                .data_map
                .get_or_insert_threadsafe(ControlSlot::default)
                .0
                .swap(true, Ordering::AcqRel)
        });
        if taken {
            // Initialised before the error: wayland-rs panics on a new_id
            // left uninitialised.
            data_init.init(id, TearingControlData(None));
            manager.post_error(
                wp_tearing_control_manager_v1::Error::TearingControlExists,
                "the surface already has a tearing control object",
            );
            return;
        }
        data_init.init(id, TearingControlData(Some(surface.downgrade())));
    }
}

impl<D> Dispatch<WpTearingControlV1, TearingControlData, D> for TearingControlState
where
    D: Dispatch<WpTearingControlV1, TearingControlData> + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _control: &WpTearingControlV1,
        request: wp_tearing_control_v1::Request,
        data: &TearingControlData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let hint = match request {
            wp_tearing_control_v1::Request::SetPresentationHint { hint } => match hint {
                WEnum::Value(hint) => hint,
                WEnum::Unknown(_) => return,
            },
            // "Destroying the object reverts the presentation hint to vsync";
            // double-buffered like any other change, so it lands on commit.
            wp_tearing_control_v1::Request::Destroy => PresentationHint::Vsync,
            _ => return,
        };
        if let Some(surface) = data.surface() {
            set_pending_hint(&surface, hint);
        }
    }

    fn destroyed(
        _state: &mut D,
        _client: ClientId,
        _control: &WpTearingControlV1,
        data: &TearingControlData,
    ) {
        let Some(surface) = data.surface() else {
            return;
        };
        with_states(&surface, |states| {
            if let Some(slot) = states.data_map.get::<ControlSlot>() {
                slot.0.store(false, Ordering::Release);
            }
        });
    }
}

fn set_pending_hint(surface: &WlSurface, hint: PresentationHint) {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<TearingHintCachedState>()
            .pending()
            .hint = hint;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_starts_out_synchronised() {
        assert!(matches!(
            TearingHintCachedState::default().hint,
            PresentationHint::Vsync
        ));
    }
}

//! What the pointer is aimed at.
//!
//! `wl_pointer` addresses one surface at a time, and a client's subsurfaces and
//! popups each own their enter/leave pair, so the target has to be the exact
//! surface the hit test landed on rather than the root of the tree it belongs
//! to. Which tree that was still matters to the compositor — a press has to
//! raise the window it landed in, and a press on a panel has to leave the
//! focused window alone — so every variant carries both halves, and a button
//! press costs no second hit test.

use std::{borrow::Cow, sync::Arc};

use smithay::{
    backend::input::InputTime,
    desktop::{LayerSurface, Window},
    input::{
        Seat,
        dnd::{DndFocus, Source},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
            PointerTarget, RelativeMotionEvent,
        },
    },
    reexports::wayland_server::{
        DisplayHandle, Resource, backend::ObjectId, protocol::wl_surface::WlSurface,
    },
    utils::{IsAlive, Logical, Point, Serial},
    wayland::seat::WaylandFocus,
};

use crate::state::State;

/// A surface under the pointer, together with the thing the compositor knows it
/// by.
// TODO: a `LockScreen` variant once `session_lock` tracks its surfaces, and an
// `X11Surface` one once XWayland lands.
#[derive(Debug, Clone, PartialEq)]
pub enum PointerFocusTarget {
    /// Somewhere inside a toplevel's tree: the window's own surface, one of its
    /// subsurfaces, or one of its popups.
    Window { window: Window, surface: WlSurface },
    /// Somewhere inside a layer surface's tree — a bar, a launcher, a wallpaper
    /// — its popups included.
    LayerShell {
        layer: LayerSurface,
        surface: WlSurface,
    },
    /// A window's own frame — its titlebar and controls. The compositor drew
    /// those pixels, so it answers for them: nothing is forwarded to the client,
    /// and the handlers in `input::decoration` do the work instead.
    ///
    /// Which *part* of the frame is deliberately absent. The pointer crossing
    /// from one control to the next would otherwise change the target's
    /// identity, and the seat would read that as leaving one window for another.
    Decoration { window: Window },
}

impl PointerFocusTarget {
    /// The client surface the events are addressed to, if any. A frame has
    /// none: its pixels belong to the compositor.
    pub fn surface(&self) -> Option<&WlSurface> {
        match self {
            Self::Window { surface, .. } | Self::LayerShell { surface, .. } => Some(surface),
            Self::Decoration { .. } => None,
        }
    }

    /// The toplevel the pointer is inside, if it is inside one — its frame
    /// included, because clicking a titlebar focuses the window under it. A
    /// layer surface answers `None`: it is not a window, and window focus has
    /// nowhere to move to when a click lands on one.
    pub fn window(&self) -> Option<&Window> {
        match self {
            Self::Window { window, .. } | Self::Decoration { window } => Some(window),
            Self::LayerShell { .. } => None,
        }
    }

    pub fn is_decoration(&self) -> bool {
        matches!(self, Self::Decoration { .. })
    }
}

impl IsAlive for PointerFocusTarget {
    fn alive(&self) -> bool {
        // Both halves answer: a client can drop the subsurface the pointer is
        // over and keep the window it hangs off very much alive.
        match self {
            Self::Window { window, surface } => window.alive() && surface.alive(),
            Self::LayerShell { layer, surface } => layer.alive() && surface.alive(),
            Self::Decoration { window } => window.alive(),
        }
    }
}

impl WaylandFocus for PointerFocusTarget {
    /// A frame answers with the window it belongs to. It is not a surface the
    /// pointer can enter, but it *is* that client's window, which is what
    /// `same_client_as` below is asking.
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Window { surface, .. } | Self::LayerShell { surface, .. } => {
                Some(Cow::Borrowed(surface))
            }
            Self::Decoration { window } => window.wl_surface(),
        }
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        self.wl_surface()
            .is_some_and(|surface| surface.id().same_client_as(object_id))
    }
}

/// Every `PointerTarget` method is the same hand-off to the focused surface.
/// Spelled out, they would be fifteen chances to forward the wrong argument to
/// the wrong call, none of which the compiler would catch.
macro_rules! delegate_to_surface {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {
        $(
            fn $method(&self, seat: &Seat<State>, data: &mut State $(, $arg: $ty)*) {
                if let Some(surface) = self.surface() {
                    PointerTarget::$method(surface, seat, data $(, $arg)*);
                }
            }
        )*
    };
}

/// Delegates to smithay's `impl PointerTarget for WlSurface`, which is what
/// actually emits `wl_pointer.enter/motion/button/axis/frame/leave` and the
/// `wp_pointer_gestures` events.
///
/// A frame has no surface, so every method here does nothing for one. That is
/// the whole of its behaviour as a *target*: it exists to withhold events from
/// the client under it. What a frame does with a click is decided in
/// [`input::decoration`](crate::input::decoration), driven from the pointer
/// handlers rather than from inside this dispatch — smithay holds the pointer's
/// own lock for the duration of these calls, so anything reaching back into the
/// seat from here would deadlock the compositor.
///
/// `replace` is left to the trait default: it does leave-old, reset the cursor
/// to the default shape, enter-new, in that order, and the reset is what stops
/// a window's custom cursor from following the pointer out of it.
impl PointerTarget<State> for PointerFocusTarget {
    delegate_to_surface! {
        enter(event: &MotionEvent);
        motion(event: &MotionEvent);
        leave(serial: Serial, time: InputTime);
        button(event: &ButtonEvent);
        relative_motion(event: &RelativeMotionEvent);
        axis(frame: AxisFrame);
        frame();
        gesture_swipe_begin(event: &GestureSwipeBeginEvent);
        gesture_swipe_update(event: &GestureSwipeUpdateEvent);
        gesture_swipe_end(event: &GestureSwipeEndEvent);
        gesture_pinch_begin(event: &GesturePinchBeginEvent);
        gesture_pinch_update(event: &GesturePinchUpdateEvent);
        gesture_pinch_end(event: &GesturePinchEndEvent);
        gesture_hold_begin(event: &GestureHoldBeginEvent);
        gesture_hold_end(event: &GestureHoldEndEvent);
    }
}

/// A drag hovers the same client surface the pointer would enter. A frame
/// accepts nothing: it has no client to offer the data to.
impl DndFocus<State> for PointerFocusTarget {
    type OfferData<S: Source> = <WlSurface as DndFocus<State>>::OfferData<S>;

    fn enter<S: Source>(
        &self,
        data: &mut State,
        dh: &DisplayHandle,
        source: Arc<S>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<Self::OfferData<S>> {
        self.surface()
            .and_then(|surface| DndFocus::enter(surface, data, dh, source, seat, location, serial))
    }

    fn motion<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        if let Some(surface) = self.surface() {
            DndFocus::motion(surface, data, offer, seat, location, time);
        }
    }

    fn leave<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
    ) {
        if let Some(surface) = self.surface() {
            DndFocus::leave(surface, data, offer, seat);
        }
    }

    fn drop<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
    ) {
        if let Some(surface) = self.surface() {
            DndFocus::drop(surface, data, offer, seat);
        }
    }
}

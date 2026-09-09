//! Interactive move and resize.
//!
//! A grab takes over the pointer for the duration of a drag: the client asked
//! for it, so the pointer belongs to the compositor until the button comes back
//! up. Both grabs work on the *floating* rect, and a tiled window is floated
//! first — dragging a tile around a layout that decides its position would just
//! snap back on the next relayout.

use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, Focus, GestureHoldBeginEvent, GestureHoldEndEvent,
        GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
        GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    utils::{Logical, Point, Rectangle, Serial, Size},
};

use crate::{layout::placement, state::State, utils::id::WindowId};

pub struct MoveGrab {
    start_data: GrabStartData<State>,
    window: WindowId,
    /// Offset from the pointer to the window's origin, so the window keeps its
    /// grip point under the cursor instead of jumping.
    offset: Point<f64, Logical>,
}

impl MoveGrab {
    pub fn new(
        start_data: GrabStartData<State>,
        window: WindowId,
        origin: Point<i32, Logical>,
    ) -> Self {
        let offset = start_data.location - origin.to_f64();
        Self {
            start_data,
            window,
            offset,
        }
    }
}

impl PointerGrab<State> for MoveGrab {
    fn motion(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(
            <State as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &MotionEvent,
    ) {
        // Focus is withheld while grabbing: the drag owns the pointer, and
        // handing enter/leave to whatever passes underneath would confuse both
        // the dragged client and the ones it crosses.
        handle.motion(state, None, event);

        let location = (event.location - self.offset).to_i32_round();
        state.shell.move_floating(self.window, location);

        // Windows-style: the preview follows the pointer, not the window, so
        // brushing an edge with the cursor is what arms a snap.
        if state.shell.track_snap(self.window, event.location) {
            state.queue_redraw();
        }
    }

    fn relative_motion(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(
            <State as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(state, None, event);
    }

    fn button(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(state, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, state, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(state, details);
    }

    fn frame(&mut self, state: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(state);
    }

    fn start_data(&self) -> &GrabStartData<State> {
        &self.start_data
    }

    /// The drag is over, however it ended — released, cancelled, or the window
    /// died under it. Landing the snap here rather than in `button` means a
    /// cancelled drag cannot leave a preview stranded on screen.
    fn unset(&mut self, state: &mut State) {
        // The layout takes the position back *before* the snap is applied, so a
        // window released onto an edge glides into it instead of jumping.
        if let Some(tile) = state.shell.tile_mut(self.window) {
            tile.release_pointer();
        }
        state.shell.release_snap(self.window);
        state.shell.refresh();
        state.queue_redraw();
    }

    // Gestures are meaningless mid-drag, but the trait needs them forwarded.
    fn gesture_swipe_begin(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(state, event);
    }
    fn gesture_swipe_update(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(state, event);
    }
    fn gesture_swipe_end(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(state, event);
    }
    fn gesture_pinch_begin(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(state, event);
    }
    fn gesture_pinch_update(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(state, event);
    }
    fn gesture_pinch_end(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(state, event);
    }
    fn gesture_hold_begin(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(state, event);
    }
    fn gesture_hold_end(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(state, event);
    }
}

pub struct ResizeGrab {
    start_data: GrabStartData<State>,
    window: WindowId,
    edges: ResizeEdge,
    /// The rect when the drag started. Deltas apply to this rather than to the
    /// running rect, so rounding cannot accumulate over a long drag.
    initial: Rectangle<i32, Logical>,
}

impl ResizeGrab {
    pub fn new(
        start_data: GrabStartData<State>,
        window: WindowId,
        edges: ResizeEdge,
        initial: Rectangle<i32, Logical>,
    ) -> Self {
        Self {
            start_data,
            window,
            edges,
            initial,
        }
    }

    fn resized(&self, location: Point<f64, Logical>) -> Rectangle<i32, Logical> {
        let delta = (location - self.start_data.location).to_i32_round::<i32>();
        let mut rect = self.initial;

        // Dragging a left or top edge moves the origin as well as the size, so
        // the opposite edge stays put.
        if matches!(
            self.edges,
            ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
        ) {
            rect.loc.x += delta.x;
            rect.size.w -= delta.x;
        }
        if matches!(
            self.edges,
            ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight
        ) {
            rect.size.w += delta.x;
        }
        if matches!(
            self.edges,
            ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
        ) {
            rect.loc.y += delta.y;
            rect.size.h -= delta.y;
        }
        if matches!(
            self.edges,
            ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight
        ) {
            rect.size.h += delta.y;
        }

        rect.size = Size::from((rect.size.w.max(1), rect.size.h.max(1)));
        rect
    }
}

impl PointerGrab<State> for ResizeGrab {
    fn motion(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(
            <State as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &MotionEvent,
    ) {
        handle.motion(state, None, event);
        let rect = self.resized(event.location);
        state.shell.resize_floating(self.window, rect);
    }

    fn relative_motion(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(
            <State as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(state, None, event);
    }

    fn button(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(state, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, state, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(state, details);
    }

    fn frame(&mut self, state: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(state);
    }

    fn start_data(&self) -> &GrabStartData<State> {
        &self.start_data
    }

    fn unset(&mut self, state: &mut State) {
        if let Some(tile) = state.shell.tile_mut(self.window) {
            tile.release_pointer();
        }
    }

    fn gesture_swipe_begin(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(state, event);
    }
    fn gesture_swipe_update(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(state, event);
    }
    fn gesture_swipe_end(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(state, event);
    }
    fn gesture_pinch_begin(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(state, event);
    }
    fn gesture_pinch_update(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(state, event);
    }
    fn gesture_pinch_end(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(state, event);
    }
    fn gesture_hold_begin(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(state, event);
    }
    fn gesture_hold_end(
        &mut self,
        state: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(state, event);
    }
}

impl State {
    /// Starts an interactive move, floating the window first if it was tiled.
    pub fn start_move(&mut self, window: &Window, serial: Serial) {
        let Some(id) = self.window_id_of(window) else {
            return;
        };
        let Some(start_data) = self.grab_start_data(serial) else {
            return;
        };

        let Some(origin) = self.begin_move(id, start_data.location) else {
            return;
        };

        if let Some(pointer) = self.wayland.seat.get_pointer() {
            pointer.set_grab(
                self,
                MoveGrab::new(start_data, id, origin),
                serial,
                Focus::Clear,
            );
        }
    }

    pub fn start_resize(&mut self, window: &Window, serial: Serial, edges: ResizeEdge) {
        let Some(id) = self.window_id_of(window) else {
            return;
        };
        let Some(start_data) = self.grab_start_data(serial) else {
            return;
        };

        let Some(initial) = self.begin_resize(id, start_data.location) else {
            return;
        };

        if let Some(pointer) = self.wayland.seat.get_pointer() {
            pointer.set_grab(
                self,
                ResizeGrab::new(start_data, id, edges, initial),
                serial,
                Focus::Clear,
            );
        }
    }

    /// Starts a move the *compositor* asked for — a drag on a titlebar.
    ///
    /// Unlike [`start_move`](Self::start_move) there is no serial to validate:
    /// the request did not come from a client, it came from a button press this
    /// compositor is in the middle of handling.
    pub fn start_frame_move(&mut self, id: WindowId, serial: Serial) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };
        let start_data = GrabStartData {
            focus: None,
            button: 0x110,
            location: pointer.current_location(),
        };

        let Some(origin) = self.begin_move(id, start_data.location) else {
            return;
        };

        pointer.set_grab(
            self,
            MoveGrab::new(start_data, id, origin),
            serial,
            Focus::Clear,
        );
    }

    /// Floats a window for a drag and hands its geometry to the pointer.
    ///
    /// Returns where the window starts, which is what keeps the grab point
    /// under the cursor.
    fn begin_move(
        &mut self,
        id: WindowId,
        grip: Point<f64, Logical>,
    ) -> Option<Point<i32, Logical>> {
        Some(self.begin_drag(id, grip)?.loc)
    }

    /// The same, for a resize: the whole rect is what the deltas apply to.
    fn begin_resize(
        &mut self,
        id: WindowId,
        grip: Point<f64, Logical>,
    ) -> Option<Rectangle<i32, Logical>> {
        self.begin_drag(id, grip)
    }

    fn begin_drag(
        &mut self,
        id: WindowId,
        grip: Point<f64, Logical>,
    ) -> Option<Rectangle<i32, Logical>> {
        // The grab arrives in global coordinates and a window's rect is local
        // to its workspace, so the two are brought into one space before
        // anything is measured between them.
        let origin = self
            .shell
            .location(id)
            .and_then(|at| self.shell.monitor_by_id(at.output))
            .map(|monitor| monitor.geometry().loc)
            .unwrap_or_default();

        self.float_for_grab(id, grip - origin.to_f64());
        let tile = self.shell.tile_mut(id)?;
        tile.follow_pointer();
        Some(tile.floating_rect())
    }

    /// Floats a window for a drag, at a rect the cursor still has hold of.
    ///
    /// A tiled window keeps the geometry it already had. A snapped or maximized
    /// one goes back to the size it had before, placed so the grab point keeps
    /// its position along the titlebar — restoring it to the rect it last
    /// floated at would drop it elsewhere on screen and leave the pointer
    /// holding nothing.
    fn float_for_grab(&mut self, id: WindowId, grip: Point<f64, Logical>) {
        let Some(tile) = self.shell.tile_mut(id) else {
            return;
        };
        if tile.state().is_floating() {
            return;
        }

        // Where it is on screen right now, which is what the grab point is
        // relative to. Read before `restore` changes anything.
        let from = tile.target();
        if !tile.state().is_tiled() {
            tile.restore();
        }

        // A window that has never floated has no other size to go back to.
        let restored = if tile.state().is_tiled() {
            from.size
        } else {
            tile.floating_rect().size
        };

        tile.set_floating_rect(placement::keep_grip(restored, from, grip));
        tile.set_state(crate::shell::tile::WindowState::Floating);
        self.shell.refresh();
    }

    pub fn window_id_of(&self, window: &Window) -> Option<WindowId> {
        use smithay::wayland::seat::WaylandFocus;
        let surface = window.wl_surface()?;
        self.shell.window_id(&surface)
    }

    /// Refuses a grab the client did not actually earn.
    ///
    /// A client can send `move`/`resize` with any serial; honouring one that does
    /// not match a real button press lets a background window hijack the pointer.
    fn grab_start_data(&self, serial: Serial) -> Option<GrabStartData<State>> {
        let pointer = self.wayland.seat.get_pointer()?;
        if !pointer.has_grab(serial) {
            return None;
        }
        pointer.grab_start_data()
    }
}

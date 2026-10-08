//! Carrying a toplevel through a drag-and-drop, for `xdg-toplevel-drag-v1`.
//!
//! The drag itself stays smithay's `DnDGrab`; this rides along with it. The
//! carried window floats and follows the cursor as under a move grab, is
//! invisible to drop-target picking so the window beneath it can be docked
//! into, and is released the way a move is when the drag ends.

use std::os::fd::OwnedFd;

use protocols::xdg_toplevel_drag::ToplevelDrag;
use smithay::{
    input::dnd::{DndAction, Source, SourceMetadata},
    reexports::wayland_server::{
        Resource,
        protocol::{
            wl_data_source::{self, WlDataSource},
            wl_surface::WlSurface,
        },
    },
    utils::{IsAlive, Logical, Point},
};

use crate::{
    shell::{Location, Shell},
    state::State,
    utils::id::WindowId,
};

/// How the drag carrying a toplevel came to an end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragEnd {
    /// Dropped onto a target that took the offer.
    Accepted,
    /// Dropped where nothing took it — the desktop, a window that declined.
    Declined,
    /// Torn down without a drop.
    Aborted,
}

/// A window riding the drag, and where the cursor holds it.
#[derive(Debug, Clone, Copy)]
struct Carried {
    id: WindowId,
    /// The cursor's position relative to the window's frame.
    grip: Point<f64, Logical>,
}

/// The `xdg_toplevel_drag_v1` behind the running drag-and-drop.
pub struct ToplevelDragSession {
    drag: ToplevelDrag,
    /// `None` until the attached toplevel maps, and again once it is unmapped
    /// to dock into a window.
    carried: Option<Carried>,
}

/// A toplevel drag's `wl_data_source`, holding back the `cancelled` that
/// smithay sends on its own when a drop finds no taker.
///
/// Chromium reads a bare `cancelled` as Escape and folds a torn-out tab back
/// into the window it came from. KWin and mutter send `dnd_drop_performed`
/// first, which is what [`State::end_toplevel_drag`] does instead.
pub struct CarriedSource {
    source: WlDataSource,
    drag: ToplevelDrag,
}

impl CarriedSource {
    pub fn new(source: WlDataSource, drag: ToplevelDrag) -> Self {
        Self { source, drag }
    }
}

impl IsAlive for CarriedSource {
    fn alive(&self) -> bool {
        self.source.alive()
    }
}

impl Source for CarriedSource {
    fn metadata(&self) -> Option<SourceMetadata> {
        Source::metadata(&self.source)
    }

    fn choose_action(&self, action: DndAction) {
        Source::choose_action(&self.source, action);
    }

    fn send(&self, mime_type: &str, fd: OwnedFd) {
        Source::send(&self.source, mime_type, fd);
    }

    fn drop_performed(&self) {
        Source::drop_performed(&self.source);
    }

    fn cancel(&self) {
        if !self.drag.is_dragging() {
            Source::cancel(&self.source);
        }
    }

    fn finished(&self) {
        Source::finished(&self.source);
    }
}

impl Shell {
    /// The window a drag is carrying, which drop-target picking looks through.
    pub fn carried_window(&self) -> Option<WindowId> {
        Some(self.toplevel_drag.as_ref()?.carried?.id)
    }

    /// Lets go of a carried window that is going away.
    pub fn forget_carried(&mut self, id: WindowId) {
        if let Some(session) = self.toplevel_drag.as_mut()
            && session.carried.is_some_and(|carried| carried.id == id)
        {
            session.carried = None;
            self.snap_previews.clear();
        }
    }

    fn active_location_at(&self, point: Point<f64, Logical>) -> Option<Location> {
        let monitor = self.monitor_at(point)?;
        Some(Location {
            output: monitor.id(),
            workspace: monitor.active().id(),
        })
    }
}

impl State {
    /// Starts carrying for a drag smithay has just taken the pointer for.
    pub fn begin_toplevel_drag(&mut self, drag: ToplevelDrag) {
        drag.start();
        self.shell.toplevel_drag = Some(ToplevelDragSession {
            drag: drag.clone(),
            carried: None,
        });
        self.carry_attached_toplevel(&drag);
    }

    /// Picks up the toplevel attached to `drag` if it is already mapped. One
    /// that is not is picked up as it maps, by [`Self::carry_mapped`].
    ///
    /// The client's offset decides the grip, as in KWin and mutter. It was
    /// measured against the rect the window had, so it is clamped into the one
    /// a maximized or tiled window floats back to.
    pub fn carry_attached_toplevel(&mut self, drag: &ToplevelDrag) {
        if self
            .shell
            .toplevel_drag
            .as_ref()
            .is_none_or(|session| session.drag != *drag)
        {
            return;
        }
        let Some(attachment) = drag.attachment() else {
            return;
        };
        let Some(id) = self
            .shell
            .xdg_shell_state
            .get_toplevel(&attachment.toplevel)
            .and_then(|toplevel| self.shell.window_id(toplevel.wl_surface()))
        else {
            return;
        };
        let Some(frame) = self.begin_drag(id, self.input.pointer_location) else {
            return;
        };
        let titlebar = self.shell.tile(id).map_or(0, |tile| tile.insets().top);
        let grip = attachment.offset + Point::from((0, titlebar));
        let grip = Point::from((grip.x.clamp(0, frame.size.w), grip.y.clamp(0, frame.size.h)));
        self.carry(id, grip.to_f64());
    }

    /// The offset `surface` is held at, when it is the toplevel a running drag
    /// is waiting on to map.
    pub fn carried_offset(&self, surface: &WlSurface) -> Option<Point<i32, Logical>> {
        let attachment = self.shell.toplevel_drag.as_ref()?.drag.attachment()?;
        let toplevel = self
            .shell
            .xdg_shell_state
            .get_toplevel(&attachment.toplevel)?;
        (toplevel.wl_surface() == surface).then_some(attachment.offset)
    }

    /// Where a window born mid-drag goes: the workspace under the cursor.
    pub fn carried_location(&self) -> Option<Location> {
        self.shell.active_location_at(self.input.pointer_location)
    }

    /// Picks up a window that has just mapped floating, under the cursor, at
    /// the offset its client attached it with.
    pub fn carry_mapped(&mut self, id: WindowId, offset: Point<i32, Logical>) {
        let titlebar = self.shell.tile(id).map_or(0, |tile| tile.insets().top);
        self.carry(id, (offset + Point::from((0, titlebar))).to_f64());
    }

    fn carry(&mut self, id: WindowId, grip: Point<f64, Logical>) {
        let Some(session) = self.shell.toplevel_drag.as_mut() else {
            return;
        };
        let previous = session.carried.replace(Carried { id, grip });
        if let Some(previous) = previous.filter(|previous| previous.id != id)
            && let Some(tile) = self.shell.tile_mut(previous.id)
        {
            tile.release_pointer();
        }
        if let Some(tile) = self.shell.tile_mut(id) {
            tile.follow_pointer();
        }
        // Chromium leaves stacking the dragged window on top to the
        // compositor, as it would under `xdg_toplevel.move`.
        self.shell.focus_window(id);
        self.carry_toplevel_to(self.input.pointer_location);
    }

    /// Follows the cursor with the carried window, onto whichever output's
    /// active workspace the cursor is over. Called on every pointer motion.
    pub fn carry_toplevel_to(&mut self, pointer: Point<f64, Logical>) {
        let Some(Carried { id, grip }) = self
            .shell
            .toplevel_drag
            .as_ref()
            .and_then(|session| session.carried)
        else {
            return;
        };
        let Some(at) = self.shell.active_location_at(pointer) else {
            return;
        };
        if self.shell.location(id).is_some_and(|current| current != at)
            && self.shell.move_tile(id, at)
        {
            self.shell.focus_window(id);
            self.queue_redraw();
        }
        let Some(origin) = self
            .shell
            .monitor_by_id(at.output)
            .map(|monitor| monitor.geometry().loc.to_f64())
        else {
            return;
        };
        self.shell
            .move_floating(id, (pointer - origin - grip).to_i32_round());
        if self.shell.track_snap(id, pointer) {
            self.queue_redraw();
        }
    }

    /// Ends the carrying, however the drag ended, and settles the window the
    /// way a finished move would: where it was let go, or into the snap zone
    /// the cursor was in.
    ///
    /// Runs from inside the pointer grab, so it must not touch the seat.
    pub fn end_toplevel_drag(&mut self, end: DragEnd) {
        let Some(session) = self.shell.toplevel_drag.take() else {
            return;
        };
        session.drag.end();
        if let Some(source) = session.drag.source() {
            if end == DragEnd::Declined
                && source.version() >= wl_data_source::EVT_DND_DROP_PERFORMED_SINCE
            {
                source.dnd_drop_performed();
            }
            if end != DragEnd::Accepted {
                source.cancelled();
            }
        }

        let Some(carried) = session.carried else {
            return;
        };
        if let Some(tile) = self.shell.tile_mut(carried.id) {
            tile.release_pointer();
        }
        // An aborted drag is reverted by the client, so nothing snaps.
        if end == DragEnd::Aborted {
            self.shell.snap_previews.clear();
        } else {
            self.shell.release_snap(carried.id);
        }
        self.shell.refresh();
        self.queue_redraw();
    }
}

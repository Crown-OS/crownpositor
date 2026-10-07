//! What the pointer does on a window's frame.
//!
//! The frame is the compositor's own drawing, so no client ever hears about
//! these events. They are routed to
//! [`PointerFocusTarget::Decoration`](crate::handlers::seat::PointerFocusTarget),
//! which forwards nothing, and `input::mouse` calls in here instead — outside
//! smithay's pointer dispatch, where taking a grab is allowed.
//!
//! The rules are the ones every desktop already has: the controls close,
//! minimize and maximize; a drag on the bar moves the window; a double-click
//! toggles maximize; a drag just outside the frame's edge resizes it.

use std::time::{Duration, Instant};

use smithay::{
    backend::input::ButtonState,
    input::pointer::CursorIcon,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    utils::{Logical, Point, Serial},
};

use crate::{
    shell::{
        WindowPart,
        decoration::{Control, TitleBarLayout},
        grab::resize_cursor,
        tile::{Tile, WindowState},
    },
    state::State,
    utils::id::WindowId,
};

/// Linux's `BTN_LEFT`. The only button a frame answers to.
pub const LEFT_BUTTON: u32 = 0x110;
/// How close together two clicks have to be to count as one double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// And how close together on screen, in logical pixels.
const DOUBLE_CLICK_SLOP: f64 = 6.0;

/// A press waiting for its release.
///
/// Recorded so a click only fires when it goes down *and* comes up on the same
/// control — dragging off a button to cancel it is behaviour every toolkit has,
/// and its absence is felt immediately.
#[derive(Debug, Clone, Copy)]
pub struct FramePress {
    pub window: WindowId,
    pub control: Control,
}

/// The last click, for spotting the second half of a double.
#[derive(Debug, Clone, Copy)]
pub struct LastClick {
    pub at: Instant,
    pub location: Point<f64, Logical>,
    pub window: WindowId,
}

impl LastClick {
    fn pairs_with(&self, window: WindowId, location: Point<f64, Logical>, now: Instant) -> bool {
        self.window == window
            && now.duration_since(self.at) <= DOUBLE_CLICK
            && (self.location - location).to_f64().x.abs() <= DOUBLE_CLICK_SLOP
            && (self.location - location).to_f64().y.abs() <= DOUBLE_CLICK_SLOP
    }
}

impl State {
    /// Lights the control under the pointer, if it is over a frame at all.
    pub fn track_frame_hover(&mut self, over_frame: bool) {
        let hovered = over_frame
            .then(|| self.control_at(self.input.pointer_location))
            .flatten();
        if self.input.hovered_control == hovered {
            return;
        }
        self.input.hovered_control = hovered;
        self.queue_pointer_redraw();
    }

    /// Shows the resize cursor over a frame's edge, and the arrow over the
    /// rest of it.
    ///
    /// Only while the pointer is on a frame: no client hears about it, so no
    /// client will ever set the cursor there. Anywhere else the image is the
    /// client's own business. Called after the seat has seen the motion,
    /// because crossing from a client onto a frame resets the cursor.
    pub fn track_frame_cursor(&mut self, over_frame: bool) {
        if !over_frame {
            return;
        }
        let icon = self
            .edge_at(self.input.pointer_location)
            .map_or(CursorIcon::Default, |(_, edge)| resize_cursor(edge));
        self.show_cursor(icon);
    }

    /// A click on a window's frame.
    ///
    /// The compositor drew those pixels, so it answers for them: nothing is
    /// forwarded to the client, and the rules are the ones every desktop has —
    /// the controls close, minimize and maximize, a drag on the bar moves the
    /// window, and a double-click toggles maximize.
    pub fn on_frame_click(&mut self, window: WindowId, state: ButtonState, serial: Serial) {
        match state {
            ButtonState::Pressed => self.press_frame(window, serial),
            ButtonState::Released => self.release_frame(window),
        }

        self.shell.refresh();
        self.update_keyboard_focus();
        self.queue_redraw();
    }

    fn press_frame(&mut self, id: WindowId, serial: Serial) {
        let location = self.input.pointer_location;

        // Clicking anywhere on a frame raises and focuses its window, controls
        // included — closing a background window should not leave the one
        // behind it thinking it never lost focus.
        self.shell.focus_window(id);

        // The band outside the frame resizes it. It never overlaps the bar, so
        // there is no control or label it could be shadowing.
        if let Some((_, edge)) = self.edge_at(location).filter(|(window, _)| *window == id) {
            self.start_frame_resize(id, edge, serial);
            return;
        }

        // A menu label opens or closes its menu, and nothing else: no drag, no
        // double-click, and the press is not remembered as one.
        if let Some((window, item)) = self.shell.menus.label_at(location) {
            self.shell.menus.toggle(window, item);
            if self.shell.menus.open().is_some() {
                self.fetch_submenu(window, item);
            }
            return;
        }

        // A press on a control only arms it. The click fires on release, and
        // only if the pointer is still on the same control — dragging off a
        // button to cancel it is behaviour every toolkit has.
        if let Some((_, control)) = self.control_at(location) {
            self.input.frame_press = Some(FramePress {
                window: id,
                control,
            });
            return;
        }

        // The bar itself. A second click in the same place toggles maximize
        // rather than starting a drag of zero length.
        let now = Instant::now();
        let doubled = self
            .input
            .last_frame_click
            .is_some_and(|last| last.pairs_with(id, location, now));

        self.input.last_frame_click = Some(LastClick {
            at: now,
            location,
            window: id,
        });

        if doubled {
            self.input.last_frame_click = None;
            self.shell.toggle_state_of(id, WindowState::Maximized);
        } else {
            self.start_frame_move(id, serial);
        }
    }

    fn release_frame(&mut self, id: WindowId) {
        let Some(press) = self.input.frame_press.take() else {
            return;
        };
        // Released somewhere else, so the click was cancelled.
        if press.window != id
            || self.control_at(self.input.pointer_location) != Some((id, press.control))
        {
            return;
        }

        match press.control {
            Control::Close => {
                if let Some(toplevel) = self.shell.tile(id).and_then(Tile::toplevel) {
                    toplevel.send_close();
                }
            }
            Control::Maximize => {
                self.shell.toggle_state_of(id, WindowState::Maximized);
            }
            // Nothing is minimized yet, so this is the nearest honest thing:
            // drop the window behind whatever else is on the workspace.
            Control::Minimize => {
                self.shell.lower_window(id);
            }
        }
    }

    /// A press inside the open menu.
    ///
    /// Separate from the frame handlers because a popup floats above every
    /// window: the click never reaches the frame it belongs to.
    pub fn on_menu_press(&mut self) {
        let location = self.input.pointer_location;

        // A click that lands on no row — the popup's own padding, or a
        // separator — is swallowed: it neither activates anything nor
        // dismisses the menu.
        let Some((window, item)) = self.shell.menus.row_at(location) else {
            return;
        };

        let submenu = self
            .open_menu_items()
            .iter()
            .find(|candidate| candidate.id == item)
            .is_some_and(|candidate| candidate.has_submenu);

        if submenu {
            self.shell.menus.descend(item);
            self.fetch_submenu(window, item);
            self.queue_redraw();
        } else {
            self.activate_menu_item(window, item);
        }
    }

    /// The pointer moved inside the open menu.
    pub fn on_menu_motion(&mut self) {
        let highlighted = self
            .shell
            .menus
            .row_at(self.input.pointer_location)
            .map(|(_, item)| item);
        if self.shell.menus.highlight(highlighted) {
            self.queue_pointer_redraw();
        }
    }

    /// Dismisses an open menu, and reports whether there was one.
    ///
    /// A click anywhere outside it closes it, which is the behaviour every
    /// menu everywhere has — and the click itself is spent doing so.
    pub fn dismiss_menu(&mut self) -> bool {
        let closed = self.shell.menus.close();
        if closed {
            self.queue_redraw();
        }
        closed
    }

    /// Which window's edge the pointer is over, if any.
    fn edge_at(&self, location: Point<f64, Logical>) -> Option<(WindowId, ResizeEdge)> {
        let hit = self.shell.window_part_under(location)?;
        match hit.part {
            WindowPart::Edge(edge) => Some((hit.id, edge)),
            WindowPart::TitleBar | WindowPart::Content => None,
        }
    }

    /// Which window's control the pointer is over, if any.
    fn control_at(&self, location: Point<f64, Logical>) -> Option<(WindowId, Control)> {
        let hit = self
            .shell
            .window_part_under(location)
            .filter(|hit| hit.part == WindowPart::TitleBar)?;
        let tile = self.shell.tile(hit.id)?;
        let layout = TitleBarLayout::new(tile.target(), tile.insets());

        // The layout is workspace-local and the pointer is global, so the hit
        // test happens in the frame's own space.
        let local = location - (hit.frame - tile.target().loc).to_f64();
        layout.control_at(local).map(|control| (hit.id, control))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(at: Instant, location: (f64, f64), window: WindowId) -> LastClick {
        LastClick {
            at,
            location: location.into(),
            window,
        }
    }

    #[test]
    fn two_quick_clicks_in_one_place_pair_up() {
        let id = WindowId::next();
        let now = Instant::now();
        assert!(click(now, (100.0, 10.0), id).pairs_with(id, (102.0, 11.0).into(), now));
    }

    #[test]
    fn a_slow_second_click_does_not() {
        let id = WindowId::next();
        let then = Instant::now();
        let now = then + DOUBLE_CLICK + Duration::from_millis(1);
        assert!(!click(then, (100.0, 10.0), id).pairs_with(id, (100.0, 10.0).into(), now));
    }

    #[test]
    fn a_second_click_somewhere_else_does_not() {
        let id = WindowId::next();
        let now = Instant::now();
        assert!(!click(now, (100.0, 10.0), id).pairs_with(id, (400.0, 10.0).into(), now));
    }

    #[test]
    fn a_second_click_on_another_window_does_not() {
        let now = Instant::now();
        let first = click(now, (100.0, 10.0), WindowId::next());
        assert!(!first.pairs_with(WindowId::next(), (100.0, 10.0).into(), now));
    }
}

//! Deciding where the menu row and its popup go, once per frame.
//!
//! A separate pass rather than something the renderer does inline, because the
//! hit test needs the same rectangles: laying out here and recording the result
//! means input and rendering read one answer instead of computing two, and a
//! menu you can click where you cannot see it is the failure mode of computing
//! them twice.
//!
//! Everything is in global logical coordinates, which is the space the pointer
//! arrives in.

use smithay::utils::{Logical, Point, Rectangle};

use crate::{
    menu::model::{ItemKind, MenuItem},
    shell::decoration::{TitleBarLayout, menu_layout},
    state::State,
    utils::id::WindowId,
};

/// The menu row is drawn in the title's colour, which is what its width has to
/// be measured against — a bold title and a regular one shape differently.
const MEASURE_COLOR: [f32; 4] = [1.0; 4];

impl State {
    /// Lays out the focused window's menu row, and the popup if one is open.
    ///
    /// Cheap when nothing has a menu, which is the overwhelmingly common case:
    /// two map lookups and a return.
    pub fn layout_menus(&mut self, scale: f64) {
        let Some(anchor) = self.menu_anchor() else {
            self.shell.menus.clear_layout();
            return;
        };

        let State { shell, text, .. } = self;

        // The row begins after the title, so the title has to be measured with
        // the same shaper that draws it. The borrow is confined to this call:
        // recording the result needs the shell back, mutably.
        let title_width = match shell.tile(anchor.window) {
            Some(tile) => text.measure(tile.title(), scale, MEASURE_COLOR, true),
            None => {
                shell.menus.clear_layout();
                return;
            }
        };
        let row_area = anchor.layout.menu(title_width);
        if row_area.size.w <= 0 {
            shell.menus.clear_layout();
            return;
        }
        let row_area = Rectangle::new(row_area.loc + anchor.origin, row_area.size);

        let Some(bar) = shell.menus.bar(anchor.window) else {
            shell.menus.clear_layout();
            return;
        };
        // Collected before anything is recorded: the labels borrow the menubar,
        // and recording needs the menus mutably.
        let labels: Vec<(i32, i32)> = bar
            .visible()
            .map(|item| {
                (
                    item.id,
                    text.measure(&item.label, scale, MEASURE_COLOR, false),
                )
            })
            .collect();
        let row = menu_layout::row(row_area, labels);

        let open = shell
            .menus
            .open()
            .and_then(|open| open.path.first().copied());
        let items: Vec<(i32, i32, bool)> = shell
            .menus
            .open_items()
            .filter(|(window, _)| *window == anchor.window)
            .map(|(_, items)| {
                items
                    .iter()
                    .filter(|item| item.visible)
                    .map(|item| {
                        (
                            item.id,
                            text.measure(&item.label, scale, MEASURE_COLOR, false),
                            item.kind == ItemKind::Separator,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let popup = (!items.is_empty())
            .then(|| menu_layout::popup(popup_anchor(&row, open, row_area), anchor.bounds, &items));

        shell.menus.record_layout(anchor.window, row, popup);
    }

    /// The items of the open popup, filtered exactly as the layout filtered
    /// them, so a row's rectangle and its label cannot drift apart.
    pub fn open_menu_items(&self) -> Vec<MenuItem> {
        self.shell
            .menus
            .open_items()
            .map(|(_, items)| items.iter().filter(|item| item.visible).cloned().collect())
            .unwrap_or_default()
    }

    /// Where the focused window's frame is, if it has one worth a menu.
    fn menu_anchor(&self) -> Option<MenuAnchor> {
        let window = self.shell.focused_window_id()?;
        let location = self.shell.location(window)?;
        let tile = self.shell.tile(window)?;
        if !tile.is_decorated() || self.shell.menus.bar(window).is_none() {
            return None;
        }

        let monitor = self.shell.monitor_by_id(location.output)?;
        Some(MenuAnchor {
            window,
            layout: TitleBarLayout::new(tile.target(), tile.insets()),
            origin: monitor.geometry().loc,
            bounds: monitor.geometry(),
        })
    }
}

struct MenuAnchor {
    window: WindowId,
    /// Workspace-local; `origin` lifts it into global coordinates.
    layout: TitleBarLayout,
    origin: Point<i32, Logical>,
    /// The output, so a popup can be kept on screen.
    bounds: Rectangle<i32, Logical>,
}

/// A popup hangs below the label that opened it, or below the row's start when
/// that label is no longer drawn — a menu whose label was squeezed out by a
/// narrowing window still has to appear somewhere sensible.
fn popup_anchor(
    row: &[menu_layout::RowEntry],
    open: Option<i32>,
    fallback: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    row.iter()
        .find(|entry| Some(entry.id) == open)
        .map(|entry| Point::from((entry.rect.loc.x, entry.rect.loc.y + entry.rect.size.h)))
        .unwrap_or_else(|| Point::from((fallback.loc.x, fallback.loc.y + fallback.size.h)))
}

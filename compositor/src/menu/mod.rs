//! Application menus in the titlebar.
//!
//! A client says where its menu lives over `org_kde_kwin_appmenu`; the
//! compositor reads it over `com.canonical.dbusmenu` and draws it. Both halves
//! are optional at every step — most applications export nothing, and the
//! titlebar simply shows a title and its controls when they do not.
//!
//! Nothing here blocks. The D-Bus work runs on the async pool and its results
//! arrive as ordinary event-loop callbacks, so a wedged client costs its own
//! menu and nothing else.

pub mod dbusmenu;
pub mod layout;
pub mod model;

use std::collections::HashMap;

use protocols::appmenu::MenuAddress;

use smithay::{
    backend::renderer::element::Id,
    utils::{Logical, Point},
};

use crate::{
    menu::model::MenuBar,
    shell::decoration::menu_layout::{Popup, RowEntry, entry_at},
    utils::id::WindowId,
};

pub use model::MenuItem;

/// Which menu is open, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenMenu {
    pub window: WindowId,
    /// The path from the menubar down to the open submenu, by item id. One
    /// entry means a top-level menu is open; two means a submenu inside it.
    pub path: Vec<i32>,
    /// The item the pointer or keyboard is on, if any.
    pub highlighted: Option<i32>,
}

/// Every window's menubar, and whichever one is open.
#[derive(Debug, Default)]
pub struct Menus {
    /// Where each window's menu lives. Kept separately from the contents: a
    /// window can name an address long before anything reads it.
    addresses: HashMap<WindowId, MenuAddress>,
    bars: HashMap<WindowId, MenuBar>,
    open: Option<OpenMenu>,
    /// Where the row and the open popup were last *drawn*, in global logical
    /// coordinates.
    ///
    /// Recorded by the render pass rather than recomputed here because the
    /// widths come from shaping the labels, and a hit test that measured them
    /// again could disagree with the picture by a pixel or a font.
    layout: Layout,
    ids: PopupIds,
}

/// The menu geometry as last drawn.
#[derive(Debug, Default)]
struct Layout {
    window: Option<WindowId>,
    row: Vec<RowEntry>,
    popup: Option<Popup>,
}

/// The popup's render elements. Generated once and kept for the compositor's
/// lifetime: at most one menu is open, so one set is all that is ever needed,
/// and stable ids are what let the damage tracker follow a popup that moves
/// from one menu to the next.
#[derive(Debug)]
struct PopupIds {
    glass: Id,
    border: Id,
    highlight: Id,
}

impl Default for PopupIds {
    fn default() -> Self {
        Self {
            glass: Id::new(),
            border: Id::new(),
            highlight: Id::new(),
        }
    }
}

impl Menus {
    pub fn address(&self, window: WindowId) -> Option<&MenuAddress> {
        self.addresses.get(&window)
    }

    /// Records where a window's menu lives. Returns the address to fetch from,
    /// or `None` if there is nothing new to do.
    pub fn set_address(
        &mut self,
        window: WindowId,
        address: Option<MenuAddress>,
    ) -> Option<MenuAddress> {
        match address {
            Some(address) => {
                if self.addresses.get(&window) == Some(&address) {
                    return None;
                }
                self.bars.remove(&window);
                self.addresses.insert(window, address.clone());
                Some(address)
            }
            None => {
                self.addresses.remove(&window);
                self.bars.remove(&window);
                self.close_for(window);
                None
            }
        }
    }

    pub fn bar(&self, window: WindowId) -> Option<&MenuBar> {
        self.bars.get(&window).filter(|bar| !bar.is_empty())
    }

    /// Stores a fetched menubar. A reply for a window that has since closed, or
    /// whose address changed, is dropped.
    pub fn set_bar(&mut self, window: WindowId, address: &MenuAddress, bar: MenuBar) -> bool {
        if self.addresses.get(&window) != Some(address) {
            return false;
        }
        self.bars.insert(window, bar);
        true
    }

    /// Fills in a submenu the user opened.
    pub fn graft(&mut self, window: WindowId, id: i32, children: Vec<MenuItem>) -> bool {
        self.bars
            .get_mut(&window)
            .is_some_and(|bar| bar.graft(id, children))
    }

    pub fn open(&self) -> Option<&OpenMenu> {
        self.open.as_ref()
    }

    /// Records where the row and popup were drawn. Called once per frame by the
    /// render pass, for the window whose frame it just built.
    pub fn record_layout(&mut self, window: WindowId, row: Vec<RowEntry>, popup: Option<Popup>) {
        self.layout = Layout {
            window: Some(window),
            row,
            popup,
        };
    }

    /// The popup as last drawn, for hit-testing and for the renderer.
    pub fn popup(&self) -> Option<&Popup> {
        self.layout.popup.as_ref()
    }

    /// The menu row as last drawn, and the window it belongs to.
    pub fn row(&self) -> Option<(WindowId, &[RowEntry])> {
        Some((self.layout.window?, self.layout.row.as_slice()))
    }

    pub fn clear_layout(&mut self) {
        self.layout = Layout::default();
    }

    pub fn glass_id(&self) -> &Id {
        &self.ids.glass
    }

    pub fn border_id(&self) -> &Id {
        &self.ids.border
    }

    pub fn highlight_id(&self) -> &Id {
        &self.ids.highlight
    }

    /// Which menu label is at a point, if the row is drawn there.
    pub fn label_at(&self, point: Point<f64, Logical>) -> Option<(WindowId, i32)> {
        let window = self.layout.window?;
        entry_at(&self.layout.row, point).map(|id| (window, id))
    }

    /// Which popup row is at a point. `None` when no menu is open, or the point
    /// is outside it.
    pub fn row_at(&self, point: Point<f64, Logical>) -> Option<(WindowId, i32)> {
        let window = self.layout.window?;
        self.layout
            .popup
            .as_ref()?
            .row_at(point)
            .map(|id| (window, id))
    }

    /// Whether a point is anywhere inside the open popup, actionable or not.
    ///
    /// A click on a separator or on the popup's padding still belongs to the
    /// menu: it must not fall through to the window underneath.
    pub fn contains(&self, point: Point<f64, Logical>) -> bool {
        self.layout
            .popup
            .as_ref()
            .is_some_and(|popup| popup.frame.to_f64().contains(point))
    }

    /// The items of whichever menu is open, ready to lay out.
    pub fn open_items(&self) -> Option<(WindowId, &[MenuItem])> {
        let open = self.open.as_ref()?;
        let bar = self.bars.get(&open.window)?;

        let mut items = bar.items.as_slice();
        for id in &open.path {
            let item = items.iter().find(|item| item.id == *id)?;
            items = item.children.as_slice();
        }
        Some((open.window, items))
    }

    /// Opens a top-level menu, or closes it if it was the one already open —
    /// which is what clicking the same label twice means everywhere.
    pub fn toggle(&mut self, window: WindowId, id: i32) -> bool {
        let already = self
            .open
            .as_ref()
            .is_some_and(|open| open.window == window && open.path.first() == Some(&id));

        self.open = (!already).then(|| OpenMenu {
            window,
            path: vec![id],
            highlighted: None,
        });
        true
    }

    /// Descends into a submenu of whatever is open.
    pub fn descend(&mut self, id: i32) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        if open.path.last() == Some(&id) {
            return false;
        }
        open.path.push(id);
        open.highlighted = None;
        true
    }

    /// Backs out one level, closing the menu entirely at the top.
    pub fn ascend(&mut self) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        open.path.pop();
        if open.path.is_empty() {
            self.open = None;
        }
        true
    }

    pub fn highlight(&mut self, id: Option<i32>) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        if open.highlighted == id {
            return false;
        }
        open.highlighted = id;
        true
    }

    pub fn close(&mut self) -> bool {
        self.open.take().is_some()
    }

    /// Drops everything a window owned. Called when it closes.
    pub fn forget(&mut self, window: WindowId) {
        self.addresses.remove(&window);
        self.bars.remove(&window);
        self.close_for(window);
    }

    fn close_for(&mut self, window: WindowId) {
        if self.open.as_ref().is_some_and(|open| open.window == window) {
            self.open = None;
        }
        if self.layout.window == Some(window) {
            self.layout = Layout::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(service: &str) -> MenuAddress {
        MenuAddress {
            service: service.to_owned(),
            path: "/MenuBar".to_owned(),
        }
    }

    fn bar(labels: &[&str]) -> MenuBar {
        MenuBar {
            items: labels
                .iter()
                .enumerate()
                .map(|(index, label)| MenuItem {
                    id: index as i32 + 1,
                    label: (*label).to_owned(),
                    ..MenuItem::default()
                })
                .collect(),
            revision: 1,
        }
    }

    #[test]
    fn a_new_address_asks_for_a_fetch_and_the_same_one_does_not() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        assert_eq!(
            menus.set_address(window, Some(address("org.app"))),
            Some(address("org.app"))
        );
        assert_eq!(menus.set_address(window, Some(address("org.app"))), None);
    }

    #[test]
    fn changing_the_address_drops_the_menu_it_had() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.set_address(window, Some(address("org.app")));
        menus.set_bar(window, &address("org.app"), bar(&["File"]));
        assert!(menus.bar(window).is_some());

        menus.set_address(window, Some(address("org.other")));
        assert!(
            menus.bar(window).is_none(),
            "the old menu is not the new one"
        );
    }

    #[test]
    fn a_reply_for_a_stale_address_is_dropped() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.set_address(window, Some(address("org.new")));
        assert!(!menus.set_bar(window, &address("org.old"), bar(&["File"])));
        assert!(menus.bar(window).is_none());
    }

    #[test]
    fn an_empty_menubar_reads_as_no_menu() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.set_address(window, Some(address("org.app")));
        menus.set_bar(window, &address("org.app"), MenuBar::default());
        assert!(menus.bar(window).is_none());
    }

    #[test]
    fn clicking_the_same_label_twice_closes_the_menu() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.toggle(window, 1);
        assert_eq!(menus.open().unwrap().path, vec![1]);

        menus.toggle(window, 1);
        assert!(menus.open().is_none());
    }

    #[test]
    fn clicking_another_label_moves_to_it() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.toggle(window, 1);
        menus.toggle(window, 2);
        assert_eq!(menus.open().unwrap().path, vec![2]);
    }

    #[test]
    fn descending_and_backing_out_walk_the_same_path() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.toggle(window, 1);
        menus.descend(10);
        assert_eq!(menus.open().unwrap().path, vec![1, 10]);

        menus.ascend();
        assert_eq!(menus.open().unwrap().path, vec![1]);

        menus.ascend();
        assert!(menus.open().is_none(), "backing out of the top closes it");
    }

    #[test]
    fn descending_into_the_open_submenu_again_does_nothing() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.toggle(window, 1);
        menus.descend(10);
        assert!(!menus.descend(10));
    }

    #[test]
    fn highlighting_reports_only_real_changes() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.toggle(window, 1);
        assert!(menus.highlight(Some(5)));
        assert!(!menus.highlight(Some(5)));
        assert!(menus.highlight(None));
    }

    #[test]
    fn the_open_menus_items_are_the_ones_at_the_end_of_the_path() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        let mut bar = bar(&["File"]);
        bar.items[0].children = vec![MenuItem {
            id: 10,
            label: "New".into(),
            ..MenuItem::default()
        }];
        menus.set_address(window, Some(address("org.app")));
        menus.set_bar(window, &address("org.app"), bar);

        menus.toggle(window, 1);
        let (_, items) = menus.open_items().expect("File is open");
        assert_eq!(items[0].label, "New");
    }

    #[test]
    fn nothing_is_open_when_no_menu_is() {
        assert!(Menus::default().open_items().is_none());
    }

    #[test]
    fn a_closing_window_takes_its_menu_with_it() {
        let mut menus = Menus::default();
        let window = WindowId::next();

        menus.set_address(window, Some(address("org.app")));
        menus.set_bar(window, &address("org.app"), bar(&["File"]));
        menus.toggle(window, 1);

        menus.forget(window);
        assert!(menus.bar(window).is_none());
        assert!(menus.address(window).is_none());
        assert!(menus.open().is_none());
    }

    /// Another window's menu closing must not close the one that is open.
    #[test]
    fn forgetting_one_window_leaves_anothers_menu_alone() {
        let mut menus = Menus::default();
        let (open, other) = (WindowId::next(), WindowId::next());

        menus.toggle(open, 1);
        menus.forget(other);
        assert!(menus.open().is_some());
    }
}

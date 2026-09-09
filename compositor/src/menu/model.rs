//! The menu tree the titlebar draws.
//!
//! `com.canonical.dbusmenu` is a loose specification: every property is
//! optional, toolkits disagree about which ones they send, and a client may
//! describe its whole menu or only the row it was asked for. So this is
//! deliberately forgiving — anything missing takes a sensible default, and an
//! item that makes no sense is dropped rather than failing the tree.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

/// How deep a menu may nest. Real menus are two or three levels; a cycle in a
/// malicious tree would otherwise recurse until the stack ran out.
const MAX_DEPTH: usize = 8;
/// How many items one level may hold.
const MAX_ITEMS: usize = 256;

/// What a row in a menu does when you click it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItemKind {
    #[default]
    Standard,
    /// A rule between groups, drawn rather than clicked.
    Separator,
    Checkbox,
    Radio,
}

/// Whether a toggleable item is on. `Unknown` is the spec's own third state,
/// and means the client has not said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Toggle {
    Off,
    On,
    #[default]
    Unknown,
}

/// One row of a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// The client's own id, which is what an activation is reported with.
    pub id: i32,
    pub label: String,
    pub kind: ItemKind,
    pub enabled: bool,
    pub visible: bool,
    pub toggle: Toggle,
    /// Populated lazily: the client is asked for a submenu's contents when the
    /// user opens it, not when its parent is drawn.
    pub children: Vec<MenuItem>,
    /// Whether the client says there is a submenu here at all.
    pub has_submenu: bool,
}

impl Default for MenuItem {
    fn default() -> Self {
        Self {
            id: 0,
            label: String::new(),
            kind: ItemKind::Standard,
            enabled: true,
            visible: true,
            toggle: Toggle::Unknown,
            children: Vec::new(),
            has_submenu: false,
        }
    }
}

impl MenuItem {
    /// Whether this row can be clicked. A separator and a greyed-out item both
    /// answer `false`, which is the only thing the caller cares about.
    pub fn is_actionable(&self) -> bool {
        self.enabled && self.visible && self.kind != ItemKind::Separator
    }
}

/// One window's menubar: the top-level row, and whatever has been opened below
/// it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuBar {
    pub items: Vec<MenuItem>,
    /// The revision the client last reported. A `LayoutUpdated` naming an older
    /// one is stale and ignored.
    pub revision: u32,
}

impl MenuBar {
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(|item| !item.visible)
    }

    /// The top-level entries, in order, that are worth drawing.
    pub fn visible(&self) -> impl Iterator<Item = &MenuItem> {
        self.items
            .iter()
            .filter(|item| item.visible && item.kind != ItemKind::Separator)
    }

    /// Replaces one submenu's contents, wherever it is in the tree.
    ///
    /// Returns whether the id was found, so a reply for a menu the user has
    /// already closed is dropped rather than grafted onto the wrong branch.
    pub fn graft(&mut self, id: i32, children: Vec<MenuItem>) -> bool {
        fn walk(items: &mut [MenuItem], id: i32, children: &mut Option<Vec<MenuItem>>) -> bool {
            for item in items {
                if item.id == id {
                    if let Some(children) = children.take() {
                        item.has_submenu = !children.is_empty();
                        item.children = children;
                    }
                    return true;
                }
                if walk(&mut item.children, id, children) {
                    return true;
                }
            }
            false
        }

        walk(&mut self.items, id, &mut Some(children))
    }
}

/// The shape `GetLayout` replies in: `(id, properties, children)`, with the
/// children boxed as variants.
pub type LayoutNode = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// Turns a `GetLayout` reply into a menubar.
///
/// The reply's root is the menubar itself and is never drawn; its children are
/// the top-level entries.
pub fn parse_layout(root: &LayoutNode) -> MenuBar {
    MenuBar {
        items: parse_children(&root.2, 0),
        revision: 0,
    }
}

fn parse_children(children: &[OwnedValue], depth: usize) -> Vec<MenuItem> {
    if depth >= MAX_DEPTH {
        return Vec::new();
    }

    children
        .iter()
        .take(MAX_ITEMS)
        .filter_map(|child| parse_node(child, depth))
        .collect()
}

fn parse_node(value: &OwnedValue, depth: usize) -> Option<MenuItem> {
    // The children arrive boxed one level deeper than the signature suggests,
    // and not every toolkit boxes them the same number of times.
    let node: LayoutNode = unbox(value)?;
    Some(parse_item(&node, depth))
}

fn parse_item(node: &LayoutNode, depth: usize) -> MenuItem {
    let (id, properties, children) = node;
    let kind = match string(properties, "type").as_deref() {
        Some("separator") => ItemKind::Separator,
        _ => match string(properties, "toggle-type").as_deref() {
            Some("checkmark") => ItemKind::Checkbox,
            Some("radio") => ItemKind::Radio,
            _ => ItemKind::Standard,
        },
    };

    let parsed = parse_children(children, depth + 1);
    MenuItem {
        id: *id,
        label: strip_mnemonics(string(properties, "label").unwrap_or_default()),
        kind,
        enabled: boolean(properties, "enabled").unwrap_or(true),
        visible: boolean(properties, "visible").unwrap_or(true),
        toggle: match integer(properties, "toggle-state") {
            Some(0) => Toggle::Off,
            Some(1) => Toggle::On,
            _ => Toggle::Unknown,
        },
        // A client that says it has a submenu but sent no children is asking to
        // be queried when the user opens it.
        has_submenu: !parsed.is_empty()
            || string(properties, "children-display").as_deref() == Some("submenu"),
        children: parsed,
    }
}

/// `&File` and `_File` both mean F is the access key. Nothing here draws the
/// underline, so the marker is simply removed — leaving it in would put a
/// stray ampersand in every menu.
fn strip_mnemonics(label: String) -> String {
    if !label.contains(['&', '_']) {
        return label;
    }

    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(char) = chars.next() {
        match char {
            // A doubled marker is a literal one.
            '&' | '_' if chars.peek() == Some(&char) => {
                out.push(char);
                chars.next();
            }
            '&' | '_' => {}
            other => out.push(other),
        }
    }
    out
}

fn unbox<'a, T: TryFrom<Value<'a>>>(value: &'a OwnedValue) -> Option<T> {
    let mut current: &Value<'a> = value;
    // Toolkits differ in how many `v` layers they wrap a child in, so unwrap
    // until something that is not a variant comes out.
    while let Value::Value(inner) = current {
        current = inner;
    }
    T::try_from(current.try_clone().ok()?).ok()
}

fn string(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    unbox::<String>(properties.get(key)?)
}

fn boolean(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    unbox::<bool>(properties.get(key)?)
}

fn integer(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    unbox::<i32>(properties.get(key)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn properties(pairs: &[(&str, OwnedValue)]) -> HashMap<String, OwnedValue> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.try_clone().unwrap()))
            .collect()
    }

    fn text(value: &str) -> OwnedValue {
        Value::from(value).try_into().unwrap()
    }

    fn node(id: i32, label: &str, children: Vec<OwnedValue>) -> LayoutNode {
        (id, properties(&[("label", text(label))]), children)
    }

    fn boxed(node: LayoutNode) -> OwnedValue {
        Value::from(node).try_into().unwrap()
    }

    #[test]
    fn the_roots_children_become_the_top_level_row() {
        let bar = parse_layout(&node(
            0,
            "",
            vec![
                boxed(node(1, "File", Vec::new())),
                boxed(node(2, "Edit", Vec::new())),
            ],
        ));

        let labels: Vec<_> = bar.visible().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, ["File", "Edit"]);
    }

    #[test]
    fn a_menu_with_no_items_reads_as_empty() {
        assert!(parse_layout(&node(0, "", Vec::new())).is_empty());
    }

    #[test]
    fn missing_properties_take_their_defaults() {
        let bar = parse_layout(&node(0, "", vec![boxed((1, HashMap::new(), Vec::new()))]));
        let item = &bar.items[0];

        assert!(item.enabled, "an item with no `enabled` is enabled");
        assert!(item.visible);
        assert_eq!(item.kind, ItemKind::Standard);
        assert_eq!(item.toggle, Toggle::Unknown);
    }

    #[test]
    fn a_separator_is_not_actionable() {
        let separator = (1, properties(&[("type", text("separator"))]), Vec::new());
        let bar = parse_layout(&node(0, "", vec![boxed(separator)]));

        assert_eq!(bar.items[0].kind, ItemKind::Separator);
        assert!(!bar.items[0].is_actionable());
        assert_eq!(bar.visible().count(), 0, "and it is not a top-level entry");
    }

    #[test]
    fn a_disabled_item_is_not_actionable() {
        let disabled = (
            1,
            properties(&[
                ("label", text("Save")),
                ("enabled", Value::from(false).try_into().unwrap()),
            ]),
            Vec::new(),
        );
        let bar = parse_layout(&node(0, "", vec![boxed(disabled)]));
        assert!(!bar.items[0].is_actionable());
    }

    #[test]
    fn toggle_types_and_states_are_read() {
        let checked = (
            1,
            properties(&[
                ("label", text("Wrap")),
                ("toggle-type", text("checkmark")),
                ("toggle-state", Value::from(1i32).try_into().unwrap()),
            ]),
            Vec::new(),
        );
        let bar = parse_layout(&node(0, "", vec![boxed(checked)]));

        assert_eq!(bar.items[0].kind, ItemKind::Checkbox);
        assert_eq!(bar.items[0].toggle, Toggle::On);
    }

    #[test]
    fn mnemonic_markers_are_stripped_but_doubled_ones_survive() {
        assert_eq!(strip_mnemonics("&File".into()), "File");
        assert_eq!(strip_mnemonics("_Edit".into()), "Edit");
        assert_eq!(strip_mnemonics("Save && Quit".into()), "Save & Quit");
        assert_eq!(strip_mnemonics("Plain".into()), "Plain");
    }

    #[test]
    fn nested_children_are_parsed() {
        let submenu = node(1, "File", vec![boxed(node(2, "New", Vec::new()))]);
        let bar = parse_layout(&node(0, "", vec![boxed(submenu)]));

        assert!(bar.items[0].has_submenu);
        assert_eq!(bar.items[0].children[0].label, "New");
    }

    /// A client that says it has a submenu but sends no children expects to be
    /// asked when the user opens it.
    #[test]
    fn a_promised_submenu_counts_even_when_empty() {
        let promise = (
            1,
            properties(&[
                ("label", text("File")),
                ("children-display", text("submenu")),
            ]),
            Vec::new(),
        );
        let bar = parse_layout(&node(0, "", vec![boxed(promise)]));

        assert!(bar.items[0].has_submenu);
        assert!(bar.items[0].children.is_empty());
    }

    #[test]
    fn a_late_reply_fills_in_the_submenu_it_was_asked_for() {
        let mut bar = parse_layout(&node(0, "", vec![boxed(node(1, "File", Vec::new()))]));

        assert!(bar.graft(
            1,
            vec![MenuItem {
                id: 2,
                label: "New".into(),
                ..MenuItem::default()
            }]
        ));
        assert_eq!(bar.items[0].children[0].label, "New");
    }

    #[test]
    fn a_reply_for_a_menu_that_is_gone_is_dropped() {
        let mut bar = parse_layout(&node(0, "", vec![boxed(node(1, "File", Vec::new()))]));
        assert!(!bar.graft(99, vec![MenuItem::default()]));
    }

    /// A tree deep enough to blow the stack has to stop somewhere, and a menu
    /// nested eight levels down is not a menu anyone can use.
    #[test]
    fn nesting_stops_at_a_sane_depth() {
        let mut deepest = node(100, "leaf", Vec::new());
        for level in 0..20 {
            deepest = node(level, "branch", vec![boxed(deepest)]);
        }

        let bar = parse_layout(&deepest);
        let mut depth = 0;
        let mut items = &bar.items;
        while let Some(first) = items.first() {
            depth += 1;
            items = &first.children;
        }
        assert!(depth <= MAX_DEPTH, "descended {depth} levels");
    }
}

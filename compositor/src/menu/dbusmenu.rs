//! The `com.canonical.dbusmenu` client.
//!
//! Every call here runs on the async pool, never on the compositor thread: a
//! menu lives in another process, and a client that is busy or wedged must not
//! be able to stall a frame. Results come back through
//! [`TaskSender`](crate::utils::runtime::TaskSender), which delivers them to the
//! event loop as closures over `&mut State`.

use zbus::{Connection, proxy};

use crate::menu::model::{LayoutNode, MenuBar, MenuItem, parse_layout};

/// How deep a `GetLayout` reply may go. `-1` is "everything", which some
/// toolkits answer with their entire menu; two levels is the whole menubar plus
/// its first row of items, and the rest is fetched when a submenu opens.
const LAYOUT_DEPTH: i32 = 2;

/// The properties worth asking for. An empty list means "all of them", which on
/// a large menu is a great deal of icon data nothing here draws.
const PROPERTIES: &[&str] = &[
    "type",
    "label",
    "enabled",
    "visible",
    "toggle-type",
    "toggle-state",
    "children-display",
];

#[proxy(
    interface = "com.canonical.dbusmenu",
    default_path = "/MenuBar",
    gen_blocking = false
)]
trait DBusMenu {
    /// `(revision, layout)`, where the layout is the root node.
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(u32, LayoutNode)>;

    /// Tells the client a menu is about to open, so it can populate it.
    /// Answers whether anything changed as a result.
    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;

    fn event(
        &self,
        id: i32,
        event_id: &str,
        data: &zbus::zvariant::Value<'_>,
        timestamp: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    fn layout_updated(&self, revision: u32, parent: i32) -> zbus::Result<()>;
}

/// Reads a window's whole menubar.
pub async fn fetch_bar(service: String, path: String) -> Option<MenuBar> {
    let proxy = connect(&service, &path).await?;
    let (revision, root) = proxy
        .get_layout(0, LAYOUT_DEPTH, PROPERTIES)
        .await
        .inspect_err(|err| tracing::debug!(%err, service, "GetLayout failed"))
        .ok()?;

    let mut bar = parse_layout(&root);
    bar.revision = revision;
    Some(bar)
}

/// Reads one submenu, for when the user opens it.
///
/// `AboutToShow` first, because a client that populates lazily has nothing to
/// hand back until it has been told the menu is opening.
pub async fn fetch_submenu(service: String, path: String, id: i32) -> Option<Vec<MenuItem>> {
    let proxy = connect(&service, &path).await?;

    // A client that does not implement this, or refuses, is not an error: the
    // menu it already described is what there is.
    let _ = proxy.about_to_show(id).await;

    let (_, root) = proxy
        .get_layout(id, LAYOUT_DEPTH, PROPERTIES)
        .await
        .inspect_err(|err| tracing::debug!(%err, service, id, "submenu GetLayout failed"))
        .ok()?;

    Some(parse_layout(&root).items)
}

/// Tells the client an item was chosen.
///
/// Fire and forget: the menu closes on the click, not on the reply, and a
/// client that never answers must not leave one open.
pub async fn activate(service: String, path: String, id: i32, timestamp: u32) {
    let Some(proxy) = connect(&service, &path).await else {
        return;
    };
    if let Err(err) = proxy
        .event(id, "clicked", &zbus::zvariant::Value::from(0i32), timestamp)
        .await
    {
        tracing::debug!(%err, service, id, "menu activation failed");
    }
}

async fn connect(service: &str, path: &str) -> Option<DBusMenuProxy<'static>> {
    let connection = Connection::session()
        .await
        .inspect_err(|err| tracing::debug!(%err, "no session bus; menus are unavailable"))
        .ok()?;

    DBusMenuProxy::builder(&connection)
        .destination(service.to_owned())
        .ok()?
        .path(path.to_owned())
        .ok()?
        .build()
        .await
        .inspect_err(|err| tracing::debug!(%err, service, path, "cannot reach the menu"))
        .ok()
}

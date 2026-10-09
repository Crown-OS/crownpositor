//! Server-side `crownos_app_icons_v1`.
//!
//! Bookkeeping only: where each application's icon is drawn, and which
//! applications were just launched from one. The compositor asks for an icon
//! when a window maps, minimizes or closes, and turns it into the spatial
//! origin of that window's motion.

mod app_id;
mod dispatch;
mod launches;

use crownos_protocols::app_icons::v1::server::{
    crownos_app_icon_manager_v1::CrownosAppIconManagerV1, crownos_app_icon_v1::CrownosAppIconV1,
};
use smithay::utils::{Logical, Rectangle};
use wayland_server::{
    Client, DisplayHandle, Resource, Weak, backend::GlobalId, protocol::wl_surface::WlSurface,
};

pub use app_id::same_app;
pub use dispatch::AppIconsDispatch;
use launches::Launches;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`AppIconsState`].
pub trait AppIconsHandler {
    fn app_icons_state(&mut self) -> &mut AppIconsState;

    /// Restores and focuses the windows of `app_id`. Returns `false` when the
    /// application has none, which makes the launcher start it.
    fn activate_app(&mut self, app_id: &str) -> bool;
}

/// Where an icon rests on its surface, in surface-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconPlacement {
    pub rect: Rectangle<i32, Logical>,
    pub corner_radius: f64,
}

/// Global data: which clients may see the manager. Activating an icon raises
/// another application's windows, so a sandboxed client must not.
pub struct AppIconsGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// User data of a `crownos_app_icon_v1`.
#[derive(Debug)]
pub struct AppIconData {
    surface: Weak<WlSurface>,
    app_id: String,
}

struct AppIcon {
    resource: CrownosAppIconV1,
    placement: Option<IconPlacement>,
}

impl AppIcon {
    fn data(&self) -> Option<&AppIconData> {
        self.resource.data::<AppIconData>()
    }
}

/// Delegate type for the `crownos_app_icon_manager_v1` global.
pub struct AppIconsState {
    global: GlobalId,
    icons: Vec<AppIcon>,
    launches: Launches,
}

impl AppIconsState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: AppIconsDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let data = AppIconsGlobalData {
            filter: Box::new(filter),
        };
        Self {
            global: display.create_global::<D, CrownosAppIconManagerV1, _>(VERSION, data),
            icons: Vec::new(),
            launches: Launches::default(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Every placed icon of `app_id`, with the surface it is drawn on.
    pub fn icons_of<'a>(
        &'a self,
        app_id: &'a str,
    ) -> impl Iterator<Item = (WlSurface, IconPlacement)> + 'a {
        self.icons.iter().filter_map(move |icon| {
            let data = icon.data()?;
            let placement = icon.placement?;
            if !same_app(&data.app_id, app_id) {
                return None;
            }
            Some((data.surface.upgrade().ok()?, placement))
        })
    }

    /// Whether `app_id` was launched from its icon moments ago. Consumes the
    /// launch, so only the application's first window grows out of the icon.
    pub fn take_launch(&mut self, app_id: &str) -> bool {
        self.launches.take(app_id)
    }

    fn insert(&mut self, resource: CrownosAppIconV1) {
        self.icons.push(AppIcon {
            resource,
            placement: None,
        });
    }

    fn place(&mut self, resource: &CrownosAppIconV1, placement: IconPlacement) {
        if let Some(icon) = self
            .icons
            .iter_mut()
            .find(|icon| icon.resource == *resource)
        {
            icon.placement = Some(placement);
        }
    }

    fn remove(&mut self, resource: &CrownosAppIconV1) {
        self.icons.retain(|icon| icon.resource != *resource);
    }
}

//! Server-side `crownos_app_icons_v1`.
//!
//! Bookkeeping only: where each application's icon is drawn. The compositor
//! asks for an icon when a window maps, minimizes or closes, and turns it into
//! the spatial origin of that window's motion. Which applications were just
//! launched it keeps itself, in [`Launches`], because only it can tell where
//! on screen a launcher's icon was at the moment it was tapped.
//!
//! Since version 2 it also tells launchers how many windows each application
//! has, and which icons a window's motion is carrying.

mod app_id;
mod dispatch;
mod launches;
mod windows;

use crownos_protocols::app_icons::v1::server::{
    crownos_app_icon_manager_v1::{self, CrownosAppIconManagerV1},
    crownos_app_icon_v1::{self, CrownosAppIconV1},
};
use smithay::utils::{Logical, Rectangle};
use wayland_server::{
    Client, DisplayHandle, Resource, Weak, backend::GlobalId, protocol::wl_surface::WlSurface,
};

pub use app_id::same_app;
pub use dispatch::AppIconsDispatch;
pub use launches::Launches;
pub use windows::AppWindows;

use windows::Published;

/// The protocol version this module speaks.
pub const VERSION: u32 = 2;

/// What the compositor provides for this protocol to be delegated to
/// [`AppIconsState`].
pub trait AppIconsHandler {
    fn app_icons_state(&mut self) -> &mut AppIconsState;

    /// Restores and focuses the windows of `app_id`, out of `icon` — the
    /// surface the activated icon is drawn on and where — when it is placed.
    /// Returns `false` when the application has none, which makes the
    /// launcher start it.
    fn activate_app(&mut self, app_id: &str, icon: Option<PlacedIcon>) -> bool;
}

/// Where an icon rests on its surface, in surface-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconPlacement {
    pub rect: Rectangle<i32, Logical>,
    pub corner_radius: f64,
}

/// One launcher icon, as a window's motion holds on to it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IconRef(CrownosAppIconV1);

/// An icon with a rectangle, and the surface it is drawn on.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedIcon {
    pub icon: IconRef,
    pub surface: WlSurface,
    pub placement: IconPlacement,
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
    lifted: bool,
}

impl AppIcon {
    fn data(&self) -> Option<&AppIconData> {
        self.resource.data::<AppIconData>()
    }

    fn speaks_v2(&self) -> bool {
        self.resource.version() >= crownos_app_icon_v1::EVT_LIFTED_SINCE
    }
}

/// Delegate type for the `crownos_app_icon_manager_v1` global.
pub struct AppIconsState {
    global: GlobalId,
    icons: Vec<AppIcon>,
    managers: Vec<CrownosAppIconManagerV1>,
    published: Published,
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
            managers: Vec::new(),
            published: Published::default(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Every placed icon of `app_id`, with the surface it is drawn on.
    pub fn icons_of<'a>(&'a self, app_id: &'a str) -> impl Iterator<Item = PlacedIcon> + 'a {
        self.icons
            .iter()
            .filter(move |icon| {
                icon.data()
                    .is_some_and(|data| same_app(&data.app_id, app_id))
            })
            .filter_map(placed)
    }

    /// Tells launchers how many windows each application has now, if that
    /// changed since they last heard.
    pub fn publish_windows(&mut self, apps: Vec<AppWindows>) {
        for app in self.published.update(apps) {
            for manager in &self.managers {
                send_windows(manager, &app);
            }
        }
    }

    /// Lifts every icon `carried` holds out of its launcher and lands every
    /// other, telling each launcher only about icons whose state changed.
    pub fn carry(&mut self, carried: &[IconRef]) {
        for icon in self.icons.iter_mut().filter(|icon| icon.speaks_v2()) {
            let lifted = carried.iter().any(|carried| carried.0 == icon.resource);
            if lifted == icon.lifted {
                continue;
            }
            icon.lifted = lifted;
            match lifted {
                true => icon.resource.lifted(),
                false => icon.resource.landed(),
            }
        }
    }

    fn placed(&self, resource: &CrownosAppIconV1) -> Option<PlacedIcon> {
        self.icons
            .iter()
            .find(|icon| icon.resource == *resource)
            .and_then(placed)
    }

    fn add_manager(&mut self, manager: CrownosAppIconManagerV1) {
        if manager.version() >= crownos_app_icon_manager_v1::EVT_APP_WINDOWS_SINCE {
            for app in self.published.all() {
                send_windows(&manager, app);
            }
            self.managers.push(manager);
        }
    }

    fn remove_manager(&mut self, manager: &CrownosAppIconManagerV1) {
        self.managers.retain(|kept| kept != manager);
    }

    fn insert(&mut self, resource: CrownosAppIconV1) {
        self.icons.push(AppIcon {
            resource,
            placement: None,
            lifted: false,
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

fn placed(icon: &AppIcon) -> Option<PlacedIcon> {
    Some(PlacedIcon {
        icon: IconRef(icon.resource.clone()),
        surface: icon.data()?.surface.upgrade().ok()?,
        placement: icon.placement?,
    })
}

fn send_windows(manager: &CrownosAppIconManagerV1, app: &AppWindows) {
    manager.app_windows(app.app_id.clone(), app.count, u32::from(app.focused));
}

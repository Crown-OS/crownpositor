//! Server-side `crownos_android_shell_v1`.
//!
//! Bookkeeping only: which xdg_toplevels are Android task windows, what each
//! committed about its task, resize lock and colour, and which splash windows
//! stand in for apps that are still starting. What a window does with those
//! details is the compositor's, told through [`AndroidShellHandler`].
//!
//! Requests are double-buffered in [`AndroidShellState`] itself and applied by
//! a pre-commit hook on the toplevel's surface, so the details are current by
//! the time the compositor's own commit handler maps the window. No
//! per-object state lives in user data, so none of it needs a lock.

mod details;
mod dispatch;
mod placeholders;

use std::time::Instant;

use crownos_protocols::android_shell::v1::server::{
    crownos_android_shell_manager_v1::CrownosAndroidShellManagerV1,
    crownos_android_toplevel_v1::CrownosAndroidToplevelV1,
};
use smithay::wayland::compositor::HookId;
use wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use wayland_server::{
    Client, DisplayHandle, Resource, backend::GlobalId, protocol::wl_surface::WlSurface,
};

pub use details::{AndroidDetails, AndroidTask, AspectRatio, ResizeLock, SizeLock, TitleColor};
pub use dispatch::AndroidShellDispatch;
pub use placeholders::{PLACEHOLDER_LIFETIME, Placeholders};

use details::BufferedDetails;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// Which part of the CrownOS Android runtime a client is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidPeer {
    /// The hwcomposer, connected through the runtime's security context.
    Hwcomposer,
    /// The runtime daemon, the only client that may mark placeholders.
    Daemon,
}

/// What the compositor provides for this protocol to be delegated to
/// [`AndroidShellState`].
pub trait AndroidShellHandler {
    fn android_shell_state(&mut self) -> &mut AndroidShellState;

    /// `toplevel` committed new Android details, or lost them with its
    /// extension (`None`).
    fn android_details_changed(&mut self, toplevel: &XdgToplevel, details: Option<&AndroidDetails>);
}

/// Names the part of the runtime a client is, or `None` for any other client.
type PeerOf = dyn Fn(&Client) -> Option<AndroidPeer> + Send + Sync;

/// Global data: which part of the runtime each client is. Only the runtime
/// sees the manager.
pub struct AndroidShellGlobalData {
    peer_of: Box<PeerOf>,
}

/// One `crownos_android_toplevel_v1` and the window it describes.
struct Extension {
    resource: CrownosAndroidToplevelV1,
    toplevel: XdgToplevel,
    surface: WlSurface,
    hook: HookId,
    details: BufferedDetails,
}

/// Delegate type for the `crownos_android_shell_manager_v1` global.
pub struct AndroidShellState {
    global: GlobalId,
    extensions: Vec<Extension>,
    placeholders: Placeholders<XdgToplevel>,
}

impl AndroidShellState {
    /// `peer_of` names the runtime's clients; every other client never sees
    /// the global. It runs while the display is busy, so it must decide from
    /// the client's own data without calling back into the display.
    pub fn new<D, F>(display: &DisplayHandle, peer_of: F) -> Self
    where
        D: AndroidShellDispatch,
        F: Fn(&Client) -> Option<AndroidPeer> + Send + Sync + 'static,
    {
        let data = AndroidShellGlobalData {
            peer_of: Box::new(peer_of),
        };
        Self {
            global: display.create_global::<D, CrownosAndroidShellManagerV1, _>(VERSION, data),
            extensions: Vec::new(),
            placeholders: Placeholders::default(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// The details `toplevel` last committed. `Some` exactly when it is an
    /// Android task window.
    pub fn details(&self, toplevel: &XdgToplevel) -> Option<&AndroidDetails> {
        self.extension_of(toplevel)
            .map(|extension| extension.details.current())
    }

    /// Tells `toplevel` the user pressed Back. `false` when it is no Android
    /// task window.
    pub fn back(&self, toplevel: &XdgToplevel) -> bool {
        let Some(extension) = self.extension_of(toplevel) else {
            return false;
        };
        extension.resource.back();
        true
    }

    /// The live placeholder waiting for `app_id`, if one was marked in the
    /// last [`PLACEHOLDER_LIFETIME`]. Consumes the mark.
    pub fn take_placeholder(&mut self, app_id: &str, now: Instant) -> Option<XdgToplevel> {
        self.placeholders
            .take(app_id, now)
            .filter(Resource::is_alive)
    }

    fn extension_of(&self, toplevel: &XdgToplevel) -> Option<&Extension> {
        self.extensions
            .iter()
            .find(|extension| extension.toplevel == *toplevel)
    }

    fn extension_on_mut(&mut self, surface: &WlSurface) -> Option<&mut Extension> {
        self.extensions
            .iter_mut()
            .find(|extension| extension.surface == *surface && extension.toplevel.is_alive())
    }

    fn extension_mut(&mut self, resource: &CrownosAndroidToplevelV1) -> Option<&mut Extension> {
        self.extensions
            .iter_mut()
            .find(|extension| extension.resource == *resource)
    }

    /// Applies what `surface`'s extension buffered, returning the toplevel and
    /// its new details when they changed.
    fn commit(&mut self, surface: &WlSurface) -> Option<(XdgToplevel, AndroidDetails)> {
        let extension = self.extension_on_mut(surface)?;
        extension.details.commit().then(|| {
            (
                extension.toplevel.clone(),
                extension.details.current().clone(),
            )
        })
    }

    fn remove(&mut self, resource: &CrownosAndroidToplevelV1) -> Option<Extension> {
        let index = self
            .extensions
            .iter()
            .position(|extension| extension.resource == *resource)?;
        Some(self.extensions.swap_remove(index))
    }
}

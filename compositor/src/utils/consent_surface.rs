//! Surfaces where the user answers a question about their own access: the
//! portal dialog and the lock screen. Synthetic input never reaches them, so
//! an agent or a remote peer cannot answer in the user's place.
//!
//! A consent dialog is a layer surface with the dialog's namespace whose
//! client runs the installed `crownos-portal-dialog`, which only root can
//! change. While one is mapped it is the only overlay drawn and hit, so no
//! other overlay can cover it.

use std::{path::Path, sync::LazyLock};

use crownos_config::schema::AppKey;
use smithay::{
    desktop::{LayerMap, LayerSurface},
    output::Output,
    reexports::wayland_server::Resource,
    wayland::shell::wlr_layer::Layer,
};

use crate::{handlers::seat::PointerFocusTarget, state::ClientState};

/// The layer-shell namespace of `crownos-portal-dialog`.
pub const PORTAL_DIALOG_NAMESPACE: &str = "crownos-portal-dialog";
/// Where xdg-desktop-portal-crownos installs the dialog.
const PORTAL_DIALOG_EXECUTABLE: &str = "/usr/lib/crownos-portal-dialog";
/// A development build of the dialog to trust as well, for nested tests.
/// Read once from the compositor's own environment.
const DIALOG_OVERRIDE: &str = "CROWN_CONSENT_DIALOG";

static DEVELOPMENT_DIALOG: LazyLock<Option<std::path::PathBuf>> =
    LazyLock::new(|| std::env::var_os(DIALOG_OVERRIDE).map(std::path::PathBuf::from));

pub fn is_consent_layer(layer: &LayerSurface) -> bool {
    is_consent_namespace(layer.namespace())
        && layer
            .wl_surface()
            .client()
            .and_then(|client| {
                client
                    .get_data::<ClientState>()
                    .and_then(|state| state.executable.clone())
            })
            .is_some_and(|executable| is_consent_executable(&executable))
}

fn is_consent_namespace(namespace: &str) -> bool {
    namespace == PORTAL_DIALOG_NAMESPACE
}

/// The installed dialog, while root controls it, or the development build
/// the compositor was started with.
fn is_consent_executable(executable: &Path) -> bool {
    if DEVELOPMENT_DIALOG.as_deref() == Some(executable) {
        return true;
    }
    let installed = AppKey::HostExecutable {
        path: executable.to_path_buf(),
    };
    executable == Path::new(PORTAL_DIALOG_EXECUTABLE) && !installed.is_session_only(None)
}

pub fn is_consent_target(target: &PointerFocusTarget) -> bool {
    match target {
        PointerFocusTarget::LayerShell { layer, .. } => is_consent_layer(layer),
        PointerFocusTarget::LockScreen { .. } => true,
        _ => false,
    }
}

/// Whether a consent dialog is mapped on `output`.
pub fn shows_consent_dialog(output: &Output) -> bool {
    smithay::desktop::layer_map_for_output(output)
        .layers()
        .any(is_consent_layer)
}

/// Whether `layer` of kind `kind` is left out while `map` holds a consent
/// dialog: every other overlay, so none can cover or imitate the dialog.
pub fn hidden_by_consent(map: &LayerMap, kind: Layer, layer: &LayerSurface) -> bool {
    kind == Layer::Overlay && !is_consent_layer(layer) && map.layers().any(is_consent_layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_portal_dialog_namespace_is_a_consent_layer() {
        assert!(is_consent_namespace("crownos-portal-dialog"));
        for namespace in ["crownbar", "crownos-portal-dialog-preview", "", "launcher"] {
            assert!(!is_consent_namespace(namespace), "{namespace}");
        }
    }

    #[test]
    fn only_the_installed_dialog_is_trusted() {
        assert!(!is_consent_executable(Path::new("/usr/bin/crownbar")));
        assert!(!is_consent_executable(Path::new(
            "/home/user/.local/bin/crownos-portal-dialog"
        )));
        let installed = Path::new(PORTAL_DIALOG_EXECUTABLE);
        let root_controls_it = !AppKey::HostExecutable {
            path: installed.to_path_buf(),
        }
        .is_session_only(None);
        assert_eq!(is_consent_executable(installed), root_controls_it);
    }
}

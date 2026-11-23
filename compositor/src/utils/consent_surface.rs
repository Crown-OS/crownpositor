//! Surfaces where the user answers a question about their own access: the
//! portal dialog and the lock screen. Synthetic input never reaches them, so
//! an agent or a remote peer cannot answer in the user's place.

use smithay::{desktop::LayerSurface, output::Output};

use crate::handlers::seat::PointerFocusTarget;

/// The layer-shell namespace of `crownos-portal-dialog`.
pub const PORTAL_DIALOG_NAMESPACE: &str = "crownos-portal-dialog";

pub fn is_consent_layer(layer: &LayerSurface) -> bool {
    is_consent_namespace(layer.namespace())
}

fn is_consent_namespace(namespace: &str) -> bool {
    namespace == PORTAL_DIALOG_NAMESPACE
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
}

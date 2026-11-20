//! 10-bit scanout, asked for per monitor with `deep_color`.
//!
//! 8-bit stays the default: 10-bit formats are the first thing broken
//! driver and panel combinations trip over, and a working desktop beats
//! deeper blacks. With `deep_color` the 10-bit formats are tried first and
//! the 8-bit ones stay behind them, so a refusal falls back instead of
//! leaving the monitor dark.

use config::{Config, OutputSetting};
use smithay::{
    backend::{allocator::Fourcc, drm::DrmNode},
    reexports::drm::control::connector,
};

use crate::{
    backend::kms::{KmsState, enable_configured_head, surface::disable_head},
    state::State,
};

const STANDARD_FORMATS: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];
const DEEP_FORMATS: &[Fourcc] = &[
    Fourcc::Abgr2101010,
    Fourcc::Xbgr2101010,
    Fourcc::Abgr8888,
    Fourcc::Argb8888,
];

/// The scanout formats to try, in order.
pub fn color_formats(deep: bool) -> &'static [Fourcc] {
    if deep { DEEP_FORMATS } else { STANDARD_FORMATS }
}

pub fn wants_deep_color(setting: Option<&OutputSetting>) -> bool {
    setting.and_then(|setting| setting.deep_color) == Some(true)
}

/// The lit heads whose `deep_color` setting differs from what their surface
/// was built with. What the hardware granted plays no part, so a panel that
/// refuses 10-bit is not rebuilt on every settings edit.
fn heads_to_rebuild(kms: &KmsState, config: &Config) -> Vec<(DrmNode, connector::Handle)> {
    kms.devices
        .iter()
        .flat_map(|(node, device)| {
            device.heads.values().filter_map(move |head| {
                let surface = device.surfaces.get(&head.state.crtc()?)?;
                let wanted =
                    wants_deep_color(config.output_setting(&head.name, head.identity.as_str()));
                (surface.deep_color != wanted).then_some((*node, head.connector))
            })
        })
        .collect()
}

/// Rebuilds each lit head whose `deep_color` setting changed, through the
/// same disable and enable a user's "off, then on" takes.
pub fn rebuild_changed_heads(state: &mut State) {
    let Some(kms) = state.backend.kms() else {
        return;
    };
    for (node, connector) in heads_to_rebuild(kms, &state.config.current) {
        disable_head(state, node, connector);
        if let Err(err) = enable_configured_head(state, node, connector) {
            tracing::error!(%err, %node, "failed to bring a monitor back at its new colour depth");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_colour_tries_ten_bits_first_and_keeps_eight_bits_behind() {
        let deep = color_formats(true);
        assert_eq!(&deep[..2], [Fourcc::Abgr2101010, Fourcc::Xbgr2101010]);
        assert_eq!(&deep[2..], color_formats(false));
    }

    #[test]
    fn only_an_explicit_choice_asks_for_deep_colour() {
        let chosen = |deep_color| OutputSetting {
            deep_color,
            ..OutputSetting::default()
        };
        assert!(wants_deep_color(Some(&chosen(Some(true)))));
        assert!(!wants_deep_color(Some(&chosen(Some(false)))));
        assert!(!wants_deep_color(Some(&chosen(None))));
        assert!(!wants_deep_color(None));
    }
}

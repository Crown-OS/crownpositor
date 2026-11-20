//! The keymap options that the user's input settings select.

use smithay::input::keyboard::XkbConfig;

const SWAP_CAPS_AND_ESC_OPTION: &str = "caps:swapescape";

/// The swap lives in the keymap, not in a keycode rewrite: the keymap that
/// clients receive then agrees with the key events, and the Caps Lock LED
/// follows the key that locks.
pub fn xkb_config(swap_caps_and_esc: bool) -> XkbConfig<'static> {
    XkbConfig {
        options: swap_caps_and_esc.then(|| SWAP_CAPS_AND_ESC_OPTION.to_owned()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_swap_selects_the_swapescape_option() {
        assert_eq!(xkb_config(true).options.as_deref(), Some("caps:swapescape"));
    }

    #[test]
    fn no_swap_selects_no_option() {
        assert_eq!(xkb_config(false).options, None);
    }
}

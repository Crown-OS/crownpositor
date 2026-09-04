//! The chord vocabulary, shared with the config file.
//!
//! [`Keybind`] is the only spelling of a shortcut CrownOS has: the settings
//! panel records one, the RON file holds it, and this translates it into what
//! the seat reports. Translating rather than re-parsing is what keeps the
//! compositor from accepting chords the panel cannot write, or rejecting ones
//! it does.

use crownos_config::{KeyCode, Keybind, Mods};
use smithay::input::keyboard::{Keysym, ModifiersState, keysyms};

/// The four modifiers a shortcut may name.
///
/// `caps_lock`, `num_lock` and `iso_level3/5_shift` are excluded: including them
/// breaks Super+Q with CapsLock on, and every Alt binding on an AltGr layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModMask {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl ModMask {
    pub fn from_smithay(modifiers: &ModifiersState) -> Self {
        Self {
            logo: modifiers.logo,
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
        }
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

impl From<Mods> for ModMask {
    fn from(mods: Mods) -> Self {
        Self {
            logo: mods.meta,
            ctrl: mods.ctrl,
            alt: mods.alt,
            shift: mods.shift,
        }
    }
}

/// A chord: some modifiers plus at most one ordinary key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: ModMask,
    /// `None` for a modifier-only chord such as `Super`, which fires on release
    /// rather than press.
    pub key: Option<Keysym>,
}

impl Chord {
    /// Binds nothing — what `keys: "None"` reads as.
    pub fn is_empty(self) -> bool {
        self.mods.is_empty() && self.key.is_none()
    }
}

impl From<Keybind> for Chord {
    fn from(bind: Keybind) -> Self {
        Self {
            mods: bind.mods.into(),
            key: bind.key.map(keysym),
        }
    }
}

/// The keysym a config key names.
///
/// Shift lives in [`ModMask`] rather than in the keysym, so a letter is its
/// lowercase symbol: `Super+Shift+Q` is `{shift}` plus `q`, which is what the
/// keymap reports for the unmodified key.
const fn keysym(key: KeyCode) -> Keysym {
    Keysym::new(match key {
        KeyCode::A => keysyms::KEY_a,
        KeyCode::B => keysyms::KEY_b,
        KeyCode::C => keysyms::KEY_c,
        KeyCode::D => keysyms::KEY_d,
        KeyCode::E => keysyms::KEY_e,
        KeyCode::F => keysyms::KEY_f,
        KeyCode::G => keysyms::KEY_g,
        KeyCode::H => keysyms::KEY_h,
        KeyCode::I => keysyms::KEY_i,
        KeyCode::J => keysyms::KEY_j,
        KeyCode::K => keysyms::KEY_k,
        KeyCode::L => keysyms::KEY_l,
        KeyCode::M => keysyms::KEY_m,
        KeyCode::N => keysyms::KEY_n,
        KeyCode::O => keysyms::KEY_o,
        KeyCode::P => keysyms::KEY_p,
        KeyCode::Q => keysyms::KEY_q,
        KeyCode::R => keysyms::KEY_r,
        KeyCode::S => keysyms::KEY_s,
        KeyCode::T => keysyms::KEY_t,
        KeyCode::U => keysyms::KEY_u,
        KeyCode::V => keysyms::KEY_v,
        KeyCode::W => keysyms::KEY_w,
        KeyCode::X => keysyms::KEY_x,
        KeyCode::Y => keysyms::KEY_y,
        KeyCode::Z => keysyms::KEY_z,

        KeyCode::Digit0 => keysyms::KEY_0,
        KeyCode::Digit1 => keysyms::KEY_1,
        KeyCode::Digit2 => keysyms::KEY_2,
        KeyCode::Digit3 => keysyms::KEY_3,
        KeyCode::Digit4 => keysyms::KEY_4,
        KeyCode::Digit5 => keysyms::KEY_5,
        KeyCode::Digit6 => keysyms::KEY_6,
        KeyCode::Digit7 => keysyms::KEY_7,
        KeyCode::Digit8 => keysyms::KEY_8,
        KeyCode::Digit9 => keysyms::KEY_9,

        KeyCode::F1 => keysyms::KEY_F1,
        KeyCode::F2 => keysyms::KEY_F2,
        KeyCode::F3 => keysyms::KEY_F3,
        KeyCode::F4 => keysyms::KEY_F4,
        KeyCode::F5 => keysyms::KEY_F5,
        KeyCode::F6 => keysyms::KEY_F6,
        KeyCode::F7 => keysyms::KEY_F7,
        KeyCode::F8 => keysyms::KEY_F8,
        KeyCode::F9 => keysyms::KEY_F9,
        KeyCode::F10 => keysyms::KEY_F10,
        KeyCode::F11 => keysyms::KEY_F11,
        KeyCode::F12 => keysyms::KEY_F12,

        KeyCode::Space => keysyms::KEY_space,
        KeyCode::Enter => keysyms::KEY_Return,
        KeyCode::Tab => keysyms::KEY_Tab,
        KeyCode::Escape => keysyms::KEY_Escape,
        KeyCode::Backspace => keysyms::KEY_BackSpace,
        KeyCode::Delete => keysyms::KEY_Delete,
        KeyCode::Insert => keysyms::KEY_Insert,
        KeyCode::Home => keysyms::KEY_Home,
        KeyCode::End => keysyms::KEY_End,
        KeyCode::PageUp => keysyms::KEY_Prior,
        KeyCode::PageDown => keysyms::KEY_Next,
        KeyCode::CapsLock => keysyms::KEY_Caps_Lock,

        KeyCode::ArrowUp => keysyms::KEY_Up,
        KeyCode::ArrowDown => keysyms::KEY_Down,
        KeyCode::ArrowLeft => keysyms::KEY_Left,
        KeyCode::ArrowRight => keysyms::KEY_Right,

        KeyCode::Minus => keysyms::KEY_minus,
        KeyCode::Equal => keysyms::KEY_equal,
        KeyCode::LeftBracket => keysyms::KEY_bracketleft,
        KeyCode::RightBracket => keysyms::KEY_bracketright,
        KeyCode::Backslash => keysyms::KEY_backslash,
        KeyCode::Semicolon => keysyms::KEY_semicolon,
        KeyCode::Quote => keysyms::KEY_apostrophe,
        KeyCode::Backquote => keysyms::KEY_grave,
        KeyCode::Comma => keysyms::KEY_comma,
        KeyCode::Period => keysyms::KEY_period,
        KeyCode::Slash => keysyms::KEY_slash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(written: &str) -> Chord {
        written
            .parse::<Keybind>()
            .unwrap_or_else(|err| panic!("`{written}`: {err}"))
            .into()
    }

    #[test]
    fn a_chord_is_read_the_way_the_file_writes_it() {
        assert_eq!(chord("Super+Shift+Q"), chord("shift+SUPER+q"));
        assert_eq!(chord("Super+Enter").key, Some(keysym(KeyCode::Enter)));
    }

    #[test]
    fn shift_stays_in_the_mask_and_letters_stay_lowercase() {
        let parsed = chord("Super+Shift+Q");

        assert!(parsed.mods.shift);
        assert_eq!(parsed.key, Some(Keysym::new(keysyms::KEY_q)));
    }

    #[test]
    fn a_modifier_only_chord_has_no_key() {
        let parsed = chord("Super+Ctrl");

        assert!(parsed.mods.logo);
        assert!(parsed.mods.ctrl);
        assert_eq!(parsed.key, None);
        assert!(!parsed.is_empty());
    }

    #[test]
    fn nothing_bound_is_an_empty_chord() {
        assert!(chord("None").is_empty());
    }

    /// The panel writes punctuation as a word, and those are the spellings that
    /// used to fall through the compositor's own parser.
    #[test]
    fn punctuation_the_panel_writes_resolves() {
        assert_eq!(
            chord("Super+Minus").key,
            Some(Keysym::new(keysyms::KEY_minus))
        );
        assert_eq!(
            chord("Super+LeftBracket").key,
            Some(Keysym::new(keysyms::KEY_bracketleft))
        );
        assert_eq!(
            chord("Super+Period").key,
            Some(Keysym::new(keysyms::KEY_period))
        );
    }

    /// Two keys sharing a keysym would make one of them unbindable.
    #[test]
    fn every_key_maps_to_a_distinct_keysym() {
        for &key in KeyCode::ALL {
            let clashes = KeyCode::ALL
                .iter()
                .filter(|&&other| keysym(other) == keysym(key));
            assert_eq!(clashes.count(), 1, "{key:?} shares its keysym");
        }
    }
}

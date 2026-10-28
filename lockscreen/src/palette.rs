use crownos_config::schema::{
    Theme,
    theme::{AccentGradient, parse_hex},
};

/// Straight RGBA in `0.0..=1.0`.
pub type Rgba = [f32; 4];

/// The toolkit's dark palette, which the lock screen always wears: it sits on
/// a dimmed picture whatever the desktop's mode. Sorted by token.
const DARK: &[(&str, u32)] = &[
    ("accent.end", 0x6d48e8ff),
    ("accent.start", 0x8f6dfbff),
    ("bar.fg", 0xfafafaff),
    ("bar.fg_hover", 0xffffffff),
    ("control.knob", 0xfafafaff),
    ("control.knob_shadow", 0x00000073),
    ("control.track", 0x3a3a3dff),
    ("frame.border", 0xffffff1f),
    ("frame.control_fill", 0xffffff26),
    ("frame.control_glyph", 0xfafafaff),
    ("frame.highlight", 0xffffff33),
    ("frame.label", 0xfafafaff),
    ("frame.tint_bottom", 0x0f0f1066),
    ("frame.tint_top", 0x0f0f1033),
    ("menu.bg", 0x1d1d1dff),
    ("menu.border", 0x2a2a2aff),
    ("menu.disabled_text", 0x5a5a60ff),
    ("menu.header_text", 0x8e8e93ff),
    ("menu.selected_bg", 0x6d48e8ff),
    ("menu.selected_text", 0xffffffff),
    ("menu.separator", 0x282828ff),
    ("menu.shortcut_text", 0x8e8e93ff),
    ("menu.text", 0xfafafaff),
    ("popover.bg", 0x1d1d1dff),
    ("popover.border", 0x282828ff),
    ("popover.hover_bg", 0x2a2a2aff),
    ("popover.muted_text", 0x9f9f9fff),
    ("popover.text", 0xfafafaff),
    ("popover.trigger_bg", 0x1c1c1cff),
    ("radio.hole", 0x1d1d1dff),
    ("radio.text", 0xfafafaff),
    ("radio.unselected_bg", 0x1c1c1cff),
    ("radio.unselected_border", 0x3e3e3eff),
    ("sidebar.bg", 0x13131400),
    ("sidebar.brand_text", 0xfafafaff),
    ("sidebar.hover_bg", 0xffffff0e),
    ("sidebar.icon", 0x8a8a93ff),
    ("sidebar.selected_bg", 0x232326ff),
    ("sidebar.selected_border", 0xffffff1c),
    ("sidebar.selected_icon", 0xfafafaff),
    ("sidebar.selected_text", 0xfafafaff),
    ("sidebar.separator", 0x262628ff),
    ("sidebar.subitem_text", 0x8a8a93ff),
    ("sidebar.text", 0xa1a1aaff),
    ("status.danger", 0xf87171ff),
    ("status.danger_bg", 0xc12b2bff),
    ("status.neutral", 0x8a8a93ff),
    ("status.on_danger", 0xffffffff),
    ("status.success", 0x4ade80ff),
    ("status.warning", 0xfbbf24ff),
    ("surface.bg", 0x0f0f10ccu32),
    ("surface.border", 0x27272aff),
    ("surface.hairline", 0x232326ff),
    ("surface.hover", 0xffffff10),
    ("surface.raised", 0x171718ff),
    ("surface.shadow", 0x00000059),
    ("surface.sunken", 0x121212ff),
    ("text.body", 0xd4d4d8ff),
    ("text.disabled", 0x5a5a60ff),
    ("text.icon", 0xd4d4d8ff),
    ("text.icon_muted", 0x8a8a93ff),
    ("text.muted", 0x9f9f9fff),
    ("text.on_accent", 0xffffffff),
    ("text.primary", 0xfafafaff),
    ("toggle_off.end", 0x2a2a2eff),
    ("toggle_off.start", 0x3f3f46ff),
];

const FALLBACK: Rgba = [1.0, 1.0, 1.0, 1.0];

/// Theme tokens resolved to colors: the theme's dark-mode edits over the
/// toolkit's dark palette.
#[derive(Debug, Clone, Copy)]
pub struct Palette<'a> {
    theme: &'a Theme,
}

impl<'a> Palette<'a> {
    pub fn new(theme: &'a Theme) -> Self {
        Self { theme }
    }

    pub fn color(&self, token: &str) -> Rgba {
        self.accent(token)
            .or_else(|| {
                self.theme
                    .slots(true)
                    .get(token)
                    .and_then(|hex| parse_hex(hex))
            })
            .or_else(|| builtin(token))
            .unwrap_or(FALLBACK)
    }

    fn accent(&self, token: &str) -> Option<Rgba> {
        let AccentGradient { start, end } = self.theme.accent.as_ref()?;
        match token {
            "accent.start" => parse_hex(start),
            "accent.end" => parse_hex(end),
            _ => None,
        }
    }
}

fn builtin(token: &str) -> Option<Rgba> {
    let index = DARK.binary_search_by(|(name, _)| name.cmp(&token)).ok()?;
    let value = DARK[index].1;
    Some([24, 16, 8, 0].map(|shift| ((value >> shift) & 0xff) as f32 / 255.0))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crownos_config::schema::theme::{COLOR_TOKENS, is_color_token};

    use super::*;

    #[test]
    fn the_table_is_sorted_and_covers_every_token() {
        assert!(DARK.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(DARK.iter().all(|(token, _)| is_color_token(token)));
        for token in COLOR_TOKENS {
            assert!(builtin(token).is_some(), "{token}");
        }
    }

    #[test]
    fn a_theme_edit_wins() {
        let theme = Theme {
            dark: BTreeMap::from([("text.primary".to_owned(), "#ff0000".to_owned())]),
            ..Theme::default()
        };
        assert_eq!(
            Palette::new(&theme).color("text.primary"),
            [1.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            Palette::new(&Theme::default()).color("text.primary"),
            builtin("text.primary").expect("listed")
        );
    }
}

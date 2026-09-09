//! Text in a titlebar: the window's title, and the menu row's labels.
//!
//! Glyphs are the one thing in a frame that no distance field will generate, so
//! they are rasterised on the CPU into a [`MemoryRenderBuffer`] and uploaded —
//! the same shape `rendering::cursor` uses for named cursor shapes, and for the
//! same reason: shaping and rasterising are far too expensive to do per frame,
//! and a title changes perhaps once a minute.
//!
//! The cache is keyed on everything that changes the pixels, so a window whose
//! title, scale and colour all stayed put costs a hash lookup per frame.

use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, fontdb,
};
use smithay::{
    backend::renderer::element::memory::MemoryRenderBuffer,
    utils::{Buffer as BufferCoords, Size, Transform},
};

/// Nominal text size in logical pixels. Line height is the usual 1.25×.
const FONT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = FONT_SIZE * 1.25;
/// Rendered text wider than this is elided; no titlebar is this wide.
const MAX_WIDTH: f32 = 4096.0;
/// Cache entries. A few dozen windows, each with a title and a menu row.
const CACHE_LIMIT: usize = 256;

/// One rasterised run of text and how wide it came out.
#[derive(Debug, Clone)]
pub struct Label {
    pub buffer: MemoryRenderBuffer,
    /// Physical pixels, so the caller can place what follows it.
    pub size: Size<i32, BufferCoords>,
}

/// What makes two rasterisations different.
///
/// The colour is in here because the glyphs are rasterised *with* it: a themed
/// or focus-faded title is a different image, not the same one drawn twice.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    text: String,
    /// Quantised so a fractional scale does not defeat the cache outright.
    scale_millis: u32,
    color: [u8; 4],
    bold: bool,
}

/// Shapes and rasterises titlebar text, caching what it produced.
pub struct TextRenderer {
    fonts: FontSystem,
    glyphs: SwashCache,
    cache: HashMap<CacheKey, Option<Label>>,
}

impl std::fmt::Debug for TextRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextRenderer")
            .field("cached", &self.cache.len())
            .finish_non_exhaustive()
    }
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    /// Loads the system's fonts. Costs a fontconfig scan once, at startup.
    pub fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
            glyphs: SwashCache::new(),
            cache: HashMap::new(),
        }
    }

    /// Builds this text at this scale and colour, or returns what it built
    /// last time.
    ///
    /// `None` — cached like any other answer — means there was nothing to draw:
    /// empty text, or a run that rasterised to no pixels at all.
    pub fn label(&mut self, text: &str, scale: f64, color: [f32; 4], bold: bool) -> Option<&Label> {
        if text.is_empty() {
            return None;
        }

        let key = CacheKey {
            text: text.to_owned(),
            scale_millis: (scale * 1000.0).round().max(1.0) as u32,
            color: color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8),
            bold,
        };

        // A cold entry rasterises; a warm one is a hash lookup. Written this
        // way rather than with the entry API because rasterising needs `&mut`
        // on two other fields at once.
        if !self.cache.contains_key(&key) {
            if self.cache.len() >= CACHE_LIMIT {
                // Titles churn as windows come and go, and an unbounded map
                // would keep every one ever seen. Nothing here is worth an LRU:
                // dropping the lot costs one re-rasterisation per live window.
                self.cache.clear();
            }
            let label = self.rasterise(&key);
            self.cache.insert(key.clone(), label);
        }

        self.cache.get(&key)?.as_ref()
    }

    fn rasterise(&mut self, key: &CacheKey) -> Option<Label> {
        let scale = key.scale_millis as f32 / 1000.0;
        let metrics = Metrics::new(FONT_SIZE * scale, LINE_HEIGHT * scale);

        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let attrs = Attrs::new().family(Family::SansSerif).weight(if key.bold {
            Weight::SEMIBOLD
        } else {
            Weight::NORMAL
        });

        {
            let mut borrowed = buffer.borrow_with(&mut self.fonts);
            borrowed.set_size(Some(MAX_WIDTH), Some(metrics.line_height));
            borrowed.set_text(&key.text, &attrs, Shaping::Advanced, None);
            borrowed.shape_until_scroll(false);
        }

        let width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
            .ceil()
            .max(1.0) as i32;
        let height = metrics.line_height.ceil().max(1.0) as i32;

        // `Argb8888` little-endian is B, G, R, A in memory, which is the order
        // the loop below writes.
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut drew = false;

        let [red, green, blue, alpha] = key.color;
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            cosmic_text::Color::rgba(red, green, blue, alpha),
            |x, y, w, h, color| {
                let coverage = color.a() as u32;
                if coverage == 0 {
                    return;
                }
                for offset_y in 0..h as i32 {
                    for offset_x in 0..w as i32 {
                        let (px, py) = (x + offset_x, y + offset_y);
                        if px < 0 || py < 0 || px >= width || py >= height {
                            continue;
                        }
                        let index = ((py * width + px) * 4) as usize;
                        // Premultiplied, which is what the renderer imports.
                        let scale = |channel: u8| (channel as u32 * coverage / 255) as u8;
                        pixels[index] = scale(color.b());
                        pixels[index + 1] = scale(color.g());
                        pixels[index + 2] = scale(color.r());
                        pixels[index + 3] = coverage as u8;
                        drew = true;
                    }
                }
            },
        );

        if !drew {
            return None;
        }

        let size = Size::from((width, height));
        Some(Label {
            buffer: MemoryRenderBuffer::from_slice(
                &pixels,
                smithay::backend::allocator::Fourcc::Argb8888,
                size,
                1,
                Transform::Normal,
                None,
            ),
            size,
        })
    }

    /// How wide this text would be, in *logical* pixels.
    ///
    /// Rasterises it if it has not been already, which is what makes the
    /// measurement and the drawing agree: a layout computed from an estimate
    /// and drawn from a shaper disagree by a few pixels per label, and a menu
    /// row is nothing but labels laid end to end.
    pub fn measure(&mut self, text: &str, scale: f64, color: [f32; 4], bold: bool) -> i32 {
        self.label(text, scale, color, bold)
            .map(|label| (label.size.w as f64 / scale).ceil() as i32)
            .unwrap_or_default()
    }

    /// Forgets everything cached. For a theme change, where every colour key
    /// went stale at once.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Adds a font the system does not have. Used by nothing yet; kept because
    /// the settings panel will want to name a UI font.
    #[allow(dead_code)]
    pub fn load_font(&mut self, data: Vec<u8>) {
        self.fonts
            .db_mut()
            .load_font_source(fontdb::Source::Binary(std::sync::Arc::new(data)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loading the system's fonts is slow enough to be worth doing once for
    /// every test in this module.
    fn renderer() -> TextRenderer {
        TextRenderer::new()
    }

    #[test]
    fn empty_text_draws_nothing() {
        let mut text = renderer();
        assert!(text.label("", 1.0, [1.0; 4], false).is_none());
    }

    #[test]
    fn a_title_rasterises_to_something_with_width() {
        let mut text = renderer();
        let Some(label) = text.label("Terminal", 1.0, [1.0; 4], true) else {
            // A machine with no fonts installed at all; nothing to assert.
            return;
        };
        assert!(label.size.w > 0 && label.size.h > 0);
    }

    #[test]
    fn a_longer_title_is_wider() {
        let mut text = renderer();
        let Some(short) = text.label("W", 1.0, [1.0; 4], false).map(|l| l.size.w) else {
            return;
        };
        let long = text
            .label("Wwwwwwwwwwww", 1.0, [1.0; 4], false)
            .map(|label| label.size.w)
            .unwrap_or_default();
        assert!(long > short, "{long} should exceed {short}");
    }

    #[test]
    fn the_same_request_twice_only_rasterises_once() {
        let mut text = renderer();
        text.label("Terminal", 1.0, [1.0; 4], false);
        let cached = text.cache.len();
        text.label("Terminal", 1.0, [1.0; 4], false);
        assert_eq!(text.cache.len(), cached);
    }

    /// Colour is part of the image, so a themed title is a separate entry —
    /// otherwise a focus change would show the old colour.
    #[test]
    fn colour_and_scale_are_part_of_the_key() {
        let mut text = renderer();
        text.label("Terminal", 1.0, [1.0, 1.0, 1.0, 1.0], false);
        text.label("Terminal", 1.0, [0.0, 0.0, 0.0, 1.0], false);
        text.label("Terminal", 2.0, [1.0, 1.0, 1.0, 1.0], false);
        assert_eq!(text.cache.len(), 3);
    }

    #[test]
    fn a_higher_scale_rasterises_larger() {
        let mut text = renderer();
        let Some(small) = text
            .label("Terminal", 1.0, [1.0; 4], false)
            .map(|l| l.size.h)
        else {
            return;
        };
        let large = text
            .label("Terminal", 2.0, [1.0; 4], false)
            .map(|label| label.size.h)
            .unwrap_or_default();
        assert!(large > small);
    }

    #[test]
    fn measuring_is_in_logical_pixels_whatever_the_scale() {
        let mut text = renderer();
        let single = text.measure("Terminal", 1.0, [1.0; 4], false);
        if single == 0 {
            // No fonts installed; nothing to compare.
            return;
        }
        let doubled = text.measure("Terminal", 2.0, [1.0; 4], false);
        // Rasterised twice as large, but reported in the same logical space —
        // within a pixel of rounding.
        assert!((doubled - single).abs() <= 2, "{single} vs {doubled}");
    }

    #[test]
    fn clearing_empties_the_cache() {
        let mut text = renderer();
        text.label("Terminal", 1.0, [1.0; 4], false);
        text.clear();
        assert!(text.cache.is_empty());
    }
}

//! XCursor themes: the format every desktop on the system already ships.
//!
//! Loaded through the `xcursor` crate, which handles the parts that are pure
//! tedium — the `index.theme` `Inherits` chain, the `XCURSOR_PATH` search
//! order, and the binary file format.

use smithay::{backend::allocator::Fourcc, input::pointer::CursorIcon};

use crate::rendering::cursor::source::{CursorSource, RawImage, shape_names};

/// The theme name to try when `XCURSOR_THEME` is unset. `xcursor` walks the
/// `Inherits` chain from here, so this reaches whatever the distribution
/// symlinked into place.
const DEFAULT_THEME: &str = "default";

pub struct XCursorSource {
    theme: xcursor::CursorTheme,
    name: String,
}

impl XCursorSource {
    /// Reads the theme name from `XCURSOR_THEME`, the same variable every
    /// toolkit reads, so the cursor matches the rest of the session without a
    /// second place to configure it.
    pub fn from_env() -> Self {
        let name = std::env::var("XCURSOR_THEME")
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| DEFAULT_THEME.to_string());
        Self::new(name)
    }

    pub fn new(name: String) -> Self {
        Self {
            theme: xcursor::CursorTheme::load(&name),
            name,
        }
    }
}

impl CursorSource for XCursorSource {
    fn describe(&self) -> String {
        format!("xcursor theme '{}'", self.name)
    }

    fn shape(&self, icon: CursorIcon, size: u32) -> Option<RawImage> {
        let path = shape_names(icon).find_map(|name| self.theme.load_icon(name))?;

        let content = std::fs::read(&path)
            .inspect_err(
                |err| tracing::warn!(%err, path = %path.display(), "unreadable cursor file"),
            )
            .ok()?;
        let images = xcursor::parser::parse_xcursor(&content)?;

        // A theme file holds several nominal sizes, and an animated cursor
        // holds several frames per size. Take the size closest to what the
        // output wants, then its first frame — animation needs a redraw clock
        // of its own, which the static shapes do not.
        let nominal = images
            .iter()
            .map(|image| image.size)
            .min_by_key(|nominal| nominal.abs_diff(size))?;
        let frame = images.into_iter().find(|image| image.size == nominal)?;

        let image = RawImage {
            // `pixels_rgba` is the file's own byte order despite the name: one
            // little-endian `0xAARRGGBB` word per pixel, premultiplied, which
            // is exactly `Argb8888`.
            pixels: frame.pixels_rgba,
            format: Fourcc::Argb8888,
            width: frame.width as i32,
            height: frame.height as i32,
            hotspot: (frame.xhot as f64, frame.yhot as f64),
        };

        if !image.is_valid() {
            tracing::warn!(path = %path.display(), "malformed cursor image, skipping");
            return None;
        }
        Some(image)
    }
}

//! XCursor themes: the format every desktop on the system already ships.
//!
//! Loaded through the `xcursor` crate, which handles the parts that are pure
//! tedium — the `index.theme` `Inherits` chain, the `XCURSOR_PATH` search
//! order, and the binary file format.

use std::time::Duration;

use smithay::{backend::allocator::Fourcc, input::pointer::CursorIcon};

use crate::rendering::cursor::source::{CursorSource, RawFrame, RawImage, RawShape, shape_names};

/// The theme name to try when neither the config nor `XCURSOR_THEME` names
/// one. `xcursor` walks the `Inherits` chain from here, so this reaches
/// whatever the distribution symlinked into place.
const DEFAULT_THEME: &str = "default";

pub struct XCursorSource {
    theme: xcursor::CursorTheme,
    name: String,
}

impl XCursorSource {
    /// `configured` wins; otherwise `XCURSOR_THEME`, the variable every toolkit
    /// reads, so the cursor matches the rest of the session.
    pub fn new(configured: Option<&str>) -> Self {
        let name = configured
            .map(str::to_owned)
            .or_else(|| {
                std::env::var("XCURSOR_THEME")
                    .ok()
                    .filter(|name| !name.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_THEME.to_owned());
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

    fn shape(&self, icon: CursorIcon, size: u32) -> Option<RawShape> {
        let path = shape_names(icon).find_map(|name| self.theme.load_icon(name))?;

        let content = std::fs::read(&path)
            .inspect_err(
                |err| tracing::warn!(%err, path = %path.display(), "unreadable cursor file"),
            )
            .ok()?;
        let images = xcursor::parser::parse_xcursor(&content)?;

        // A theme file holds several nominal sizes, and an animated cursor
        // holds several frames per size: every frame of the closest size.
        let nominal = images
            .iter()
            .map(|image| image.size)
            .min_by_key(|nominal| nominal.abs_diff(size))?;
        let frames = images
            .into_iter()
            .filter(|image| image.size == nominal)
            .map(|frame| RawFrame {
                delay: Duration::from_millis(frame.delay.into()),
                image: RawImage {
                    // `pixels_rgba` is the file's own byte order despite the
                    // name: one little-endian `0xAARRGGBB` word per pixel,
                    // premultiplied, which is exactly `Argb8888`.
                    pixels: frame.pixels_rgba,
                    format: Fourcc::Argb8888,
                    width: frame.width as i32,
                    height: frame.height as i32,
                    hotspot: (frame.xhot as f64, frame.yhot as f64),
                },
            })
            .collect();

        let shape = RawShape::new(frames);
        if shape.is_none() {
            tracing::warn!(path = %path.display(), "malformed cursor image, skipping");
        }
        shape
    }
}

//! The seam every cursor theme format plugs into.
//!
//! A cursor theme does exactly one thing for us: turn a shape name and a pixel
//! size into pixels plus a hot point. [`CursorSource`] is that, and nothing
//! else — no caching, no scale arithmetic, no render elements, because those are
//! identical whatever the format and live in the parent module.
//!
//! Adding a format is therefore: implement this trait, and register it in
//! [`Cursor::sources`]. Nothing downstream changes.
//!
//! [`Cursor::sources`]: super::Cursor::sources

use smithay::{backend::allocator::Fourcc, input::pointer::CursorIcon};

/// One rasterised cursor image, in its own pixel space.
///
/// Deliberately dumb: a source hands back pixels at whatever size it could
/// actually produce — themes ship discrete sizes and rarely the one asked for —
/// and the caller reconciles that with the output scale.
pub struct RawImage {
    /// Premultiplied, `format`-ordered, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
    /// How to read `pixels`. Formats differ by source: XCursor files are
    /// `Argb8888`, a `tiny-skia` pixmap is `Abgr8888`.
    pub format: Fourcc,
    pub width: i32,
    pub height: i32,
    /// The hot point in *this image's* pixels, from its top-left corner.
    pub hotspot: (f64, f64),
}

impl RawImage {
    /// Whether this is something a renderer can actually import. Sources are
    /// parsing files off disk, so a truncated or absurd image is a real
    /// possibility and must not reach the renderer.
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.pixels.len() == (self.width as usize) * (self.height as usize) * 4
    }
}

/// A place cursor shapes come from.
///
/// Sources are consulted in order and the first hit wins, so a source is free
/// to know only some shapes — returning `None` simply defers to the next one.
pub trait CursorSource {
    /// Identifies this source in logs. Include the theme name: "no cursor
    /// found" is only actionable if you know where we looked.
    fn describe(&self) -> String;

    /// The shape for `icon`, rasterised as close to `size` pixels square as
    /// this source can manage.
    fn shape(&self, icon: CursorIcon, size: u32) -> Option<RawImage>;
}

/// The shape names to try for an icon, most standard first.
///
/// The w3c name comes first, then the legacy X11 spellings: themes predating
/// the CSS names only ship `left_ptr`, themes following them only ship
/// `default`, and plenty of themes in between ship one shape under both. Every
/// name-addressed format needs the same list, so it lives here rather than in
/// any one source.
pub fn shape_names(icon: CursorIcon) -> impl Iterator<Item = &'static str> {
    // Collected rather than chained lazily: `alt_names` borrows the icon, and a
    // list this short is not worth threading a lifetime through the trait for.
    let mut names = vec![icon.name()];
    names.extend_from_slice(icon.alt_names());
    names.into_iter()
}

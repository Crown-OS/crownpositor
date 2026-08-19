//! The cursor of last resort: an arrow drawn from a path, no files involved.
//!
//! A machine with no cursor theme installed is a normal state — a fresh install,
//! a container, a headless test box — and a compositor whose pointer is
//! invisible is unusable. So this source always answers, for every shape.
//! It is registered last, so it only ever draws what the real themes could not.

use smithay::{backend::allocator::Fourcc, input::pointer::CursorIcon};
use tiny_skia::{FillRule, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::rendering::cursor::source::{CursorSource, RawImage};

/// A left-pointing arrow in the unit square, so one path serves every size:
/// the tip, down the vertical left edge, around the tail, and back up the
/// diagonal.
const ARROW: &[(f32, f32)] = &[
    (0.00, 0.00),
    (0.00, 0.72),
    (0.19, 0.55),
    (0.30, 0.85),
    (0.42, 0.79),
    (0.31, 0.49),
    (0.52, 0.49),
];

/// Below this the outline swallows the shape.
const MIN_SIZE: u32 = 12;

pub struct BuiltinSource;

impl CursorSource for BuiltinSource {
    fn describe(&self) -> String {
        "the built-in arrow".to_string()
    }

    /// The same arrow for every icon. Drawing a distinct shape per icon would
    /// mean maintaining a font's worth of paths for a case that only comes up
    /// when the session has no theme at all.
    fn shape(&self, _icon: CursorIcon, size: u32) -> Option<RawImage> {
        draw_arrow(size.max(MIN_SIZE))
    }
}

fn draw_arrow(size: u32) -> Option<RawImage> {
    let mut pixmap = Pixmap::new(size, size)?;

    // The outline is centred on the path, so the tip needs half of it — round
    // up to a whole pixel — of slack to stay inside the buffer.
    let width = (size as f32 / 16.0).max(1.0);
    let inset = width;
    let span = size as f32 - 2.0 * inset;

    let mut builder = PathBuilder::new();
    for (index, (x, y)) in ARROW.iter().enumerate() {
        let (x, y) = (inset + x * span, inset + y * span);
        if index == 0 {
            builder.move_to(x, y);
        } else {
            builder.line_to(x, y);
        }
    }
    builder.close();
    let path = builder.finish()?;

    // White fill, black outline: readable on a dark desktop and a light one,
    // which is the whole reason cursors are drawn this way.
    let mut fill = Paint::default();
    fill.set_color_rgba8(255, 255, 255, 255);
    fill.anti_alias = true;
    pixmap.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);

    let mut outline = Paint::default();
    outline.set_color_rgba8(0, 0, 0, 255);
    outline.anti_alias = true;
    pixmap.stroke_path(
        &path,
        &outline,
        &Stroke {
            width,
            line_join: LineJoin::Miter,
            ..Default::default()
        },
        Transform::identity(),
        None,
    );

    Some(RawImage {
        // tiny-skia stores premultiplied `[r, g, b, a]` bytes, which as a
        // little-endian word is `0xAABBGGRR` — `Abgr8888`, not `Argb8888`.
        pixels: pixmap.data().to_vec(),
        format: Fourcc::Abgr8888,
        width: size as i32,
        height: size as i32,
        // The tip, which the inset moved off the corner.
        hotspot: (inset as f64, inset as f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrow_rasterises_at_every_plausible_size() {
        for size in [MIN_SIZE, 24, 32, 48, 96, 256] {
            let image = draw_arrow(size).expect("the built-in cursor must always be drawable");
            assert!(image.is_valid(), "size {size} produced a malformed image");
            // The hot point is the tip, so it belongs within a stroke width of
            // the corner — a hotspot in the middle of the arrow would make the
            // pointer click somewhere other than where it looks.
            let slack = (size as f64 / 16.0).max(1.0) + 0.5;
            assert!(image.hotspot.0 <= slack && image.hotspot.1 <= slack);
        }
    }

    #[test]
    fn an_absurd_cursor_size_still_produces_an_image() {
        // `XCURSOR_SIZE=1` is nonsense, but it has to degrade rather than panic
        // inside tiny-skia or hand back a zero-sized buffer.
        let image = BuiltinSource
            .shape(CursorIcon::Default, 1)
            .expect("the floor must keep the arrow drawable");
        assert!(image.is_valid());
        assert_eq!(image.width, MIN_SIZE as i32);
    }

    #[test]
    fn every_icon_is_answered() {
        // The last source in the chain must never defer, or the pointer
        // vanishes for whichever shape a client happened to ask for.
        for icon in [
            CursorIcon::Default,
            CursorIcon::Text,
            CursorIcon::Grabbing,
            CursorIcon::NwseResize,
        ] {
            assert!(BuiltinSource.shape(icon, 24).is_some());
        }
    }
}

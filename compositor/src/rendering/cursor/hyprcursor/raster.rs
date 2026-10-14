//! Turns one frame file of a shape archive into premultiplied pixels.

use resvg::{
    tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform},
    usvg,
};

use crate::rendering::cursor::hyprcursor::meta::Resize;

pub fn vector(data: &[u8], side: u32) -> Option<Pixmap> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default())
        .inspect_err(|err| tracing::warn!(%err, "unparsable hyprcursor svg"))
        .ok()?;
    let (width, height) = (tree.size().width(), tree.size().height());
    // Fit and centre, as librsvg's viewport does for libhyprcursor: a frame
    // that is not quite square keeps its proportions.
    let scale = side as f32 / width.max(height);
    let offset_x = (side as f32 - width * scale) / 2.0;
    let offset_y = (side as f32 - height * scale) / 2.0;

    let mut pixmap = Pixmap::new(side, side)?;
    resvg::render(
        &tree,
        Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y),
        &mut pixmap.as_mut(),
    );
    Some(pixmap)
}

pub fn bitmap(data: &[u8], side: u32, resize: Resize) -> Option<Pixmap> {
    let source = Pixmap::decode_png(data)
        .inspect_err(|err| tracing::warn!(%err, "undecodable hyprcursor png"))
        .ok()?;
    if source.width() == side && source.height() == side {
        return Some(source);
    }

    let quality = match resize {
        Resize::Bilinear => FilterQuality::Bilinear,
        Resize::Nearest | Resize::None => FilterQuality::Nearest,
    };
    let mut pixmap = Pixmap::new(side, side)?;
    pixmap.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &PixmapPaint {
            quality,
            ..PixmapPaint::default()
        },
        Transform::from_scale(
            side as f32 / source.width() as f32,
            side as f32 / source.height() as f32,
        ),
        None,
    );
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#fff"/></svg>"##;

    #[test]
    fn svg_fills_the_requested_side() {
        let pixmap = vector(SQUARE, 30).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (30, 30));
        assert_eq!(pixmap.pixel(15, 15).unwrap().alpha(), 255);
    }

    #[test]
    fn png_is_resampled_to_the_requested_side() {
        let mut source = Pixmap::new(4, 4).unwrap();
        source.fill(resvg::tiny_skia::Color::WHITE);
        let png = source.encode_png().unwrap();
        let pixmap = bitmap(&png, 8, Resize::Bilinear).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (8, 8));
        assert_eq!(pixmap.pixel(4, 4).unwrap().alpha(), 255);
    }

    #[test]
    fn garbage_is_refused_rather_than_panicking() {
        assert!(vector(b"not svg", 24).is_none());
        assert!(bitmap(b"not png", 24, Resize::None).is_none());
    }
}

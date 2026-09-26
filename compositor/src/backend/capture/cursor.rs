//! Cursor metadata for sessions in `metadata` cursor mode: where the hot point
//! is on the captured image, and what the cursor looks like, sent only when
//! either changes.

use smithay::{
    backend::allocator::Fourcc,
    input::pointer::CursorImageStatus,
    utils::{Buffer as BufferCoords, Logical, Point, Rectangle, SealedFile, Size},
};

use protocols::crownos_screencast::{CursorImage, ScreencastSession};

use crate::rendering::cursor::{Cursor, source::RawImage};

/// What the client was last told.
#[derive(Debug, Default)]
pub struct CursorReport {
    status: Option<CursorImageStatus>,
    hotspot: Point<i32, BufferCoords>,
    position: Option<Point<i32, BufferCoords>>,
}

impl CursorReport {
    pub fn update(
        &mut self,
        session: &ScreencastSession,
        cursor: &Cursor,
        output: Rectangle<i32, Logical>,
        scale: f64,
        pointer: Point<f64, Logical>,
    ) {
        if self.status.as_ref() != Some(&cursor.status) {
            self.hotspot = send_shape(session, cursor, scale);
            self.status = Some(cursor.status.clone());
            self.position = None;
        }

        let position = on_output(output, pointer).then(|| {
            let local = (pointer - output.loc.to_f64()).to_physical(scale);
            Point::<i32, BufferCoords>::from((local.x.round() as i32, local.y.round() as i32))
        });
        if position == self.position {
            return;
        }
        match position {
            Some(position) => session.cursor_position(position, self.hotspot),
            None => session.cursor_leave(),
        }
        self.position = position;
    }
}

fn on_output(output: Rectangle<i32, Logical>, pointer: Point<f64, Logical>) -> bool {
    output.to_f64().contains(pointer)
}

/// Sends the current shape and returns its hot point in buffer pixels.
///
/// A client's own cursor surface is reported as having no image: its pixels
/// live in a client buffer the capture path does not read back.
fn send_shape(
    session: &ScreencastSession,
    cursor: &Cursor,
    scale: f64,
) -> Point<i32, BufferCoords> {
    let image = match &cursor.status {
        CursorImageStatus::Named(icon) => {
            cursor.raw_image(*icon, Cursor::buffer_scale(scale.into()))
        }
        CursorImageStatus::Hidden | CursorImageStatus::Surface(_) => None,
    };
    let Some(image) = image else {
        session.cursor_shape(None);
        return Point::default();
    };

    let hotspot = Point::from((
        image.hotspot.0.round() as i32,
        image.hotspot.1.round() as i32,
    ));
    let pixels = argb8888(image);
    match SealedFile::with_data(c"crownos-screencast-cursor", &pixels.bytes) {
        Ok(file) => session.cursor_shape(Some(CursorImage {
            pixels: std::os::fd::AsFd::as_fd(&file),
            size: pixels.size,
            stride: pixels.size.w * 4,
            hotspot,
        })),
        Err(err) => {
            tracing::warn!(%err, "failed to share the cursor image");
            session.cursor_shape(None);
        }
    }
    hotspot
}

struct Pixels {
    bytes: Vec<u8>,
    size: Size<u32, BufferCoords>,
}

/// The image in the byte order the protocol promises. Themes hand out either
/// ARGB8888 or, from the built-in rasteriser, ABGR8888.
fn argb8888(image: RawImage) -> Pixels {
    let size = Size::from((image.width.max(0) as u32, image.height.max(0) as u32));
    let mut bytes = image.pixels;
    if image.format == Fourcc::Abgr8888 {
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Pixels { bytes, size }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abgr_is_swizzled_and_argb_left_alone() {
        let abgr = RawImage {
            pixels: vec![1, 2, 3, 4],
            format: Fourcc::Abgr8888,
            width: 1,
            height: 1,
            hotspot: (0.0, 0.0),
        };
        assert_eq!(argb8888(abgr).bytes, vec![3, 2, 1, 4]);

        let argb = RawImage {
            pixels: vec![1, 2, 3, 4],
            format: Fourcc::Argb8888,
            width: 1,
            height: 1,
            hotspot: (0.0, 0.0),
        };
        let pixels = argb8888(argb);
        assert_eq!(pixels.bytes, vec![1, 2, 3, 4]);
        assert_eq!(pixels.size, Size::from((1, 1)));
    }

    #[test]
    fn the_far_edge_of_the_output_is_off_it() {
        let output = Rectangle::new((1920, 0).into(), (1280, 1024).into());
        assert!(on_output(output, (1920.0, 0.0).into()));
        assert!(!on_output(output, (3200.0, 10.0).into()));
        assert!(!on_output(output, (1919.5, 10.0).into()));
    }
}

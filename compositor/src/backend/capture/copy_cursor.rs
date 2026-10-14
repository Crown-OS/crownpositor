//! Cursor sessions of `ext-image-copy-capture-v1`: where the pointer is over
//! a source, and what it looks like, as its own small image.

use std::time::Duration;

use smithay::{
    backend::allocator::Fourcc,
    input::pointer::CursorImageStatus,
    reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    utils::{Buffer as BufferCoords, Logical, Point, Size, Transform},
    wayland::{
        image_copy_capture::{BufferConstraints, CaptureFailureReason, CursorSession, Frame},
        shm::with_buffer_contents_mut,
    },
};

use crate::{
    backend::capture::{
        copy::SHM_FORMATS,
        source::{CaptureSource, Placement},
    },
    rendering::cursor::{Cursor, source::RawImage},
    utils::pixels::copy_swapping_red_blue,
};

pub struct CopyCursor {
    pub session: CursorSession,
    pub source: CaptureSource,
    pub pending: Vec<Frame>,
    /// The shape, at the scale it was drawn for, whose hot point and size
    /// the client was last told.
    announced: Option<(CursorImageStatus, f64)>,
    image: Option<RawImage>,
    /// The shape the client's latest frame shows.
    delivered: Option<CursorImageStatus>,
}

impl CopyCursor {
    pub fn new(session: CursorSession, source: CaptureSource) -> Self {
        Self {
            session,
            source,
            pending: Vec::new(),
            announced: None,
            image: None,
            delivered: None,
        }
    }

    pub fn update(
        &mut self,
        cursor: &Cursor,
        placement: &Placement<'_>,
        pointer: Point<f64, Logical>,
        now: Duration,
    ) {
        let scale = placement.scale();
        let shape = (cursor.status.clone(), scale);
        if self.announced.as_ref() != Some(&shape) {
            self.image = shape_image(cursor, &cursor.status, scale);
            let (size, hotspot) = self.image.as_ref().map_or(((1, 1), (0, 0)), |image| {
                (
                    (image.width, image.height),
                    (
                        image.hotspot.0.round() as i32,
                        image.hotspot.1.round() as i32,
                    ),
                )
            });
            let size = Size::<i32, BufferCoords>::from(size);
            if self
                .session
                .current_constraints()
                .map(|current| current.size)
                != Some(size)
            {
                self.session.update_constraints(cursor_constraints(size));
                for frame in self.pending.drain(..) {
                    frame.fail(CaptureFailureReason::BufferConstraints);
                }
            }
            self.session.set_cursor_hotspot(hotspot);
            self.announced = Some(shape);
        }

        let position =
            (placement.on_screen && placement.area.to_f64().contains(pointer)).then(|| {
                let local = (pointer - placement.area.loc.to_f64()).to_physical(scale);
                Point::<i32, BufferCoords>::from((local.x.round() as i32, local.y.round() as i32))
            });
        self.session.set_cursor_pos(position);

        if self.delivered.as_ref() == Some(&cursor.status) || self.pending.is_empty() {
            return;
        }
        let frame = self.pending.remove(0);
        if write_image(&frame.buffer(), self.image.as_ref()) {
            frame.success(Transform::Normal, None, now);
            self.delivered = Some(cursor.status.clone());
        } else {
            frame.fail(CaptureFailureReason::BufferConstraints);
        }
    }
}

pub fn cursor_constraints(size: Size<i32, BufferCoords>) -> BufferConstraints {
    BufferConstraints {
        size,
        shm: SHM_FORMATS.to_vec(),
        dma: None,
    }
}

/// The image of a named shape. A client's own cursor surface has no pixels
/// here, so it shows as nothing, like a hidden cursor.
pub fn shape_image(cursor: &Cursor, status: &CursorImageStatus, scale: f64) -> Option<RawImage> {
    match status {
        CursorImageStatus::Named(icon) => cursor.raw_image(*icon, scale.into()),
        CursorImageStatus::Hidden | CursorImageStatus::Surface(_) => None,
    }
}

/// Clears the buffer and writes the image into its top-left corner.
fn write_image(buffer: &WlBuffer, image: Option<&RawImage>) -> bool {
    with_buffer_contents_mut(buffer, |ptr, len, data| {
        // Safety: smithay hands out the pool's mapping for the duration of
        // this closure, and `len` is its length.
        let memory = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        let offset = data.offset as usize;
        let stride = data.stride as usize;
        let Some(pixels) = memory.get_mut(offset..offset + stride * data.height as usize) else {
            return false;
        };
        pixels.fill(0);
        let Some(image) = image else {
            return true;
        };
        let row = image.width as usize * 4;
        if image.width > data.width || image.height > data.height {
            return false;
        }
        for (dst, src) in pixels
            .chunks_exact_mut(stride)
            .zip(image.pixels.chunks_exact(row))
        {
            let dst = &mut dst[..row];
            match image.format {
                Fourcc::Abgr8888 => copy_swapping_red_blue(dst, src),
                _ => dst.copy_from_slice(src),
            }
        }
        true
    })
    .unwrap_or(false)
}

//! `ext-image-copy-capture-v1`: one frame at a time into a buffer the client
//! hands over. A dmabuf is drawn into directly; shm is drawn on the GPU and
//! read back. A frame completes once its source changed since the session's
//! previous frame, so a client that keeps asking is paced by the screen.

use std::{collections::HashMap, time::Duration};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, ExportMem, Offscreen, Texture,
            gles::{GlesRenderer, GlesTexture},
        },
    },
    output::Output,
    reexports::wayland_server::{
        Resource, backend::ObjectId, protocol::wl_buffer::WlBuffer, protocol::wl_shm,
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform},
    wayland::{
        dmabuf::get_dmabuf,
        image_copy_capture::{
            BufferConstraints, CaptureFailureReason, DmabufConstraints, Frame, Session,
        },
        shm::with_buffer_contents_mut,
    },
};

use protocols::crownos_screencast::CursorMode;

use crate::{
    backend::capture::{
        render::{Canvas, CaptureError, CaptureRenderer, DrawTarget, Effects, Scene, draw_scene},
        session::RenderTargets,
        source::{CaptureSource, Placement},
    },
    utils::pixels::copy_swapping_red_blue,
};

/// `wl_shm` guarantees both, and they share one memory layout.
pub const SHM_FORMATS: [wl_shm::Format; 2] = [wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888];

pub struct CopySession {
    pub session: Session,
    pub source: CaptureSource,
    pub shown_on: Option<Output>,
    pub dirty: bool,
    pub pending: Vec<Frame>,
    pub copier: Copier,
}

pub enum CopyOutcome {
    /// Nothing changed since the previous frame; the frame waits.
    Unchanged,
    /// The source changed size: new constraints went out.
    Reconfigured,
    Copied(Vec<Rectangle<i32, BufferCoords>>),
}

impl CopySession {
    pub fn new(session: Session, source: CaptureSource) -> Self {
        Self {
            session,
            shown_on: match &source {
                CaptureSource::Output(output) => Some(output.clone()),
                CaptureSource::Window(_) | CaptureSource::Workspace(_) => None,
            },
            source,
            dirty: true,
            pending: Vec::new(),
            copier: Copier::default(),
        }
    }

    /// Draws into the oldest pending frame's buffer, sending new constraints
    /// instead when the source changed size.
    pub fn copy<R>(
        &mut self,
        renderer: &mut R,
        scene: Scene<'_>,
        dma: Option<DmabufConstraints>,
    ) -> Result<CopyOutcome, CaptureError>
    where
        R: CaptureRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        let size = image_size(&scene.placement);
        let buffer_size = Size::<i32, BufferCoords>::from((size.w, size.h));
        if self
            .session
            .current_constraints()
            .map(|current| current.size)
            != Some(buffer_size)
        {
            self.session
                .update_constraints(constraints(buffer_size, dma));
            for frame in self.pending.drain(..) {
                frame.fail(CaptureFailureReason::BufferConstraints);
            }
            return Ok(CopyOutcome::Reconfigured);
        }
        let Some(frame) = self.pending.first() else {
            return Ok(CopyOutcome::Unchanged);
        };
        let cursor_mode = if self.session.draw_cursor() {
            CursorMode::Embedded
        } else {
            CursorMode::Hidden
        };
        let drawn = self.copier.draw(
            renderer,
            scene,
            &frame.buffer(),
            cursor_mode,
            Transform::Normal,
            None,
            false,
        )?;
        Ok(drawn.map_or(CopyOutcome::Unchanged, CopyOutcome::Copied))
    }
}

/// Draws a source into client buffers, keeping what it needs between frames
/// to redraw only what changed.
#[derive(Default)]
pub struct Copier {
    pub targets: RenderTargets,
    image: Option<(Size<i32, Physical>, Transform)>,
    /// What shm frames are drawn into before they are read back.
    readback: Option<GlesTexture>,
    /// Frames the damage tracker has drawn, and the frame each target last
    /// received, so a buffer the client cycles back in is only redrawn where
    /// it is stale.
    drawn: u64,
    last_drawn: HashMap<ObjectId, u64>,
    readback_drawn: Option<u64>,
}

impl Copier {
    /// Draws `scene`, turned by `transform`, into `buffer`: a dmabuf
    /// directly, shm through a GPU readback of `crop` (buffer coordinates)
    /// or of the whole image. Returns the damage, or `None` when nothing
    /// changed since the buffer's previous contents — unless `force`, which
    /// fills the buffer regardless.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<R>(
        &mut self,
        renderer: &mut R,
        scene: Scene<'_>,
        buffer: &WlBuffer,
        cursor_mode: CursorMode,
        transform: Transform,
        crop: Option<Rectangle<i32, BufferCoords>>,
        force: bool,
    ) -> Result<Option<Vec<Rectangle<i32, BufferCoords>>>, CaptureError>
    where
        R: CaptureRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        let scale = scene.placement.scale();
        let size = image_size(&scene.placement);
        let turned = transform.invert().transform_size(size);
        let image = Size::<i32, BufferCoords>::from((turned.w, turned.h));
        if self.image != Some((size, transform)) || self.targets.damage.is_none() {
            *self = Self {
                image: Some((size, transform)),
                ..Self::default()
            };
            self.targets.reset(size, scale, transform);
        }

        let canvas = Canvas {
            size,
            scale,
            transform,
        };
        let RenderTargets {
            damage,
            blur,
            corners,
            canvases,
            ..
        } = &mut self.targets;
        let Some(tracker) = damage.as_mut() else {
            return Ok(None);
        };
        let effects = Effects {
            blur,
            corners,
            canvases,
        };

        let next = self.drawn + 1;
        self.drawn = next;
        let age = |last: Option<&u64>| last.map_or(0, |last| (next - last) as usize);
        match get_dmabuf(buffer).cloned() {
            Ok(dmabuf) => {
                let age = age(self.last_drawn.get(&buffer.id()));
                self.last_drawn.insert(buffer.id(), next);
                let drawn = draw_scene(
                    renderer,
                    tracker,
                    effects,
                    cursor_mode,
                    scene,
                    canvas,
                    DrawTarget::Dmabuf(dmabuf),
                    age,
                )?;
                if let Some((_, sync)) = &drawn
                    && sync.wait().is_err()
                {
                    tracing::warn!("interrupted waiting for a copy-capture render");
                }
                Ok(drawn
                    .map(|(damage, _)| damage)
                    .or_else(|| force.then(Vec::new)))
            }
            Err(_) => {
                let mut texture = readback_texture(renderer.gles(), &mut self.readback, image)?;
                let age = age(self.readback_drawn.as_ref());
                self.readback_drawn = Some(next);
                let drawn = draw_scene(
                    renderer,
                    tracker,
                    effects,
                    cursor_mode,
                    scene,
                    canvas,
                    DrawTarget::Texture(&mut texture),
                    age,
                )?;
                let damage = match drawn {
                    Some((damage, _)) => damage,
                    None if force => Vec::new(),
                    None => return Ok(None),
                };
                let crop = crop.unwrap_or(Rectangle::from_size(image));
                read_into_shm(renderer.gles(), &mut texture, buffer, crop)?;
                Ok(Some(damage))
            }
        }
    }
}

/// Completes the oldest pending frame.
pub fn complete(
    session: &mut CopySession,
    damage: Vec<Rectangle<i32, BufferCoords>>,
    presented: Duration,
) {
    if !session.pending.is_empty() {
        session
            .pending
            .remove(0)
            .success(Transform::Normal, Some(damage), presented);
    }
}

pub fn image_size(placement: &Placement<'_>) -> Size<i32, Physical> {
    placement
        .area
        .size
        .to_physical_precise_round(placement.scale())
}

pub fn constraints(
    size: Size<i32, BufferCoords>,
    dma: Option<DmabufConstraints>,
) -> BufferConstraints {
    BufferConstraints {
        size,
        shm: SHM_FORMATS.to_vec(),
        dma,
    }
}

fn readback_texture(
    gles: &mut GlesRenderer,
    cached: &mut Option<GlesTexture>,
    size: Size<i32, BufferCoords>,
) -> Result<GlesTexture, CaptureError> {
    if let Some(texture) = cached.as_ref().filter(|texture| texture.size() == size) {
        return Ok(texture.clone());
    }
    let texture: GlesTexture =
        Offscreen::<GlesTexture>::create_buffer(gles, Fourcc::Abgr8888, size)
            .map_err(|err| CaptureError::Bind(err.to_string()))?;
    *cached = Some(texture.clone());
    Ok(texture)
}

/// Reads `crop` of the drawn image back and writes it into the client's shm
/// buffer, which may be larger but never smaller.
fn read_into_shm(
    gles: &mut GlesRenderer,
    texture: &mut GlesTexture,
    buffer: &WlBuffer,
    crop: Rectangle<i32, BufferCoords>,
) -> Result<(), CaptureError> {
    let size = crop.size;
    let framebuffer = gles
        .bind(texture)
        .map_err(|err| CaptureError::Bind(err.to_string()))?;
    let mapping = gles
        .copy_framebuffer(&framebuffer, crop, Fourcc::Abgr8888)
        .map_err(|err| CaptureError::Render(err.to_string()))?;
    drop(framebuffer);
    let pixels = gles
        .map_texture(&mapping)
        .map_err(|err| CaptureError::Render(err.to_string()))?;

    let row = size.w as usize * 4;
    let rows = size.h as usize;
    let written = with_buffer_contents_mut(buffer, |ptr, len, data| {
        // Safety: smithay hands out the pool's mapping for the duration of
        // this closure, and `len` is its length.
        let memory = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        // Top row first, whatever `TextureMapping::flipped` says: smithay
        // draws into offscreen textures upside down already, so that they
        // sample upright, and that cancels out GL's bottom-up readback.
        (0..rows).all(|y| {
            let start = data.offset as usize + y * data.stride as usize;
            match (
                memory.get_mut(start..start + row),
                pixels.get(y * row..(y + 1) * row),
            ) {
                (Some(dst), Some(src)) => {
                    copy_swapping_red_blue(dst, src);
                    true
                }
                _ => false,
            }
        })
    });
    match written {
        Ok(true) => Ok(()),
        _ => Err(CaptureError::Render("the shm buffer is too small".into())),
    }
}

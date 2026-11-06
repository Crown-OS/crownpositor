//! The lock screen's pixels, per output: laid out when something changed,
//! and only the sprites whose content did rasterised again.

use std::{collections::HashMap, io::Read, path::Path, sync::Arc, time::Instant};

use crownos_config::schema::LockBackground;
use lockscreen::{
    clock::MORPH_BLUR,
    image::{background_zoom, cover, decode},
    layout::{Content, Live, Sprite},
    lockscene::{AssetId, Limits},
    tiny_skia::Pixmap,
    transition::LAYER_BLUR_REACH,
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::element::{Id, memory::MemoryRenderBuffer},
    },
    utils::Transform,
};

use crate::{
    shell::lock_screen::{OutputPicture, OutputVisuals, PreparedSprite},
    state::{
        State,
        lock_screen::{DecodedImages, depth::hides_clock},
    },
    utils::id::OutputId,
};

/// A picture source: a file the user named, or an image a plugin sent.
enum Source {
    Path(String),
    Asset(Arc<[u8]>),
}

impl State {
    pub(super) fn prepare_lock_visuals(&mut self) {
        let outputs: Vec<(OutputId, f64, [f32; 2])> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| {
                let size = monitor.geometry().size;
                (
                    monitor.id(),
                    monitor.output().current_scale().fractional_scale(),
                    [size.w as f32, size.h as f32],
                )
            })
            .collect();
        let Some(lock) = self.shell.lock_screen.active_mut() else {
            return;
        };
        let current = |visuals: Option<&OutputVisuals>, scale: f64, size: [f32; 2]| {
            visuals.is_some_and(|visuals| visuals.scale == scale && visuals.size == size)
        };
        let stale = lock.dirty
            || lock.visuals.len() != outputs.len()
            || outputs
                .iter()
                .any(|(output, scale, size)| !current(lock.visuals.get(output), *scale, *size));
        let Some(painter) = self.lock.painter.as_mut() else {
            return;
        };
        if !stale {
            return;
        }

        let scene = self.lock.scene.current();
        let zoom = background_zoom(&scene.config.background);
        let notice = lock.flow.notice();
        let notice_text = notice.text(Instant::now());
        let live = Live {
            face: &self.lock.face,
            user: self.lock.user.as_ref().filter(|_| scene.config.show_user),
            password_chars: lock.flow.password_chars(),
            checking: lock.flow.is_checking(),
            notice: notice_text.as_deref().map(|text| (text, notice.is_error())),
            caps_lock: lock.caps_lock,
        };
        lock.clock_warp = scene.config.clock.frame.warp;
        let now = Instant::now();

        let mut previous = std::mem::take(&mut lock.visuals);
        for (output, scale, size) in outputs {
            let mut old = previous.remove(&output).unwrap_or_default();
            let same_scale = old.scale == scale;
            let mut leaving = std::mem::take(&mut old.leaving);
            let mut sprites = Vec::new();
            for sprite in painter.layout(scene, live, &self.lock.images, size) {
                let morph_reach = if sprite.clock_group().is_some() {
                    MORPH_BLUR
                } else {
                    0.0
                };
                let reach = LAYER_BLUR_REACH + morph_reach + sprite.reach();
                let pad = (reach * scale as f32).ceil() as u32;
                if let Some(reused) = same_scale
                    .then(|| take_unchanged(&mut old.sprites, &sprite, pad))
                    .flatten()
                {
                    sprites.push(reused);
                    continue;
                }
                let replaced = same_scale
                    .then(|| take_replaced_glyph(&mut old.sprites, &sprite))
                    .flatten();
                let Some(pixmap) =
                    painter.paint(&sprite.as_coverage(), scale as f32, pad, &self.lock.images)
                else {
                    continue;
                };
                let mut prepared = prepare(sprite, pad, &pixmap);
                if let Some(replaced) = replaced {
                    leaving.push((replaced, now));
                    prepared.born = Some(now);
                }
                sprites.push(prepared);
            }
            let pixels = size.map(|side| (f64::from(side) * scale).round() as u32);
            let mut subject = old
                .subject
                .filter(|subject| subject.picture.pixels == pixels);
            if let Some(subject) = &mut subject {
                subject.hides_clock = hides_clock(subject, &sprites, size, zoom);
            }
            lock.visuals.insert(
                output,
                OutputVisuals {
                    scale,
                    size,
                    sprites,
                    leaving,
                    picture: old.picture.filter(|picture| picture.pixels == pixels),
                    subject,
                },
            );
        }
        lock.dirty = false;
    }

    /// Decodes the background picture and any plugin images off-thread, for
    /// the lock that is up now.
    pub(super) fn load_lock_pictures(&mut self) {
        let scene = self.lock.scene.current();
        let background = match &scene.config.background {
            LockBackground::Image(src) => match AssetId::from_src(src) {
                Some(id) => self.lock.scene.asset(&id).map(Source::Asset),
                None => Some(Source::Path(src.clone())),
            },
            LockBackground::Desktop | LockBackground::Token(_) => None,
        };
        let background_id = match &scene.config.background {
            LockBackground::Image(src) => AssetId::from_src(src),
            _ => None,
        };
        let images: Vec<(AssetId, Arc<[u8]>)> = scene
            .referenced_assets()
            .into_iter()
            .filter(|id| Some(id) != background_id.as_ref())
            .filter_map(|id| self.lock.scene.asset(&id).map(|bytes| (id, bytes)))
            .collect();
        let sizes = self.lock_output_pixels();
        let generation = self.lock.generation;
        self.common.tasks.run(
            async move {
                tokio::task::spawn_blocking(move || decode_all(background, &sizes, images))
                    .await
                    .unwrap_or_default()
            },
            move |state, decoded| state.lock_pictures_ready(generation, decoded),
        );
    }

    /// Every output's size in pixels, for pictures to cover.
    pub(super) fn lock_output_pixels(&self) -> Vec<(OutputId, [u32; 2])> {
        self.shell
            .monitors()
            .iter()
            .map(|monitor| {
                let scale = monitor.output().current_scale().fractional_scale();
                let size = monitor
                    .geometry()
                    .size
                    .to_physical_precise_round::<_, i32>(scale);
                (monitor.id(), [size.w as u32, size.h as u32])
            })
            .collect()
    }

    fn lock_pictures_ready(&mut self, generation: u64, decoded: Decoded) {
        if generation != self.lock.generation {
            return;
        }
        let Some(lock) = self.shell.lock_screen.active_mut() else {
            return;
        };
        for (output, pixmap) in decoded.pictures {
            lock.visuals.entry(output).or_default().picture = Some(output_picture(&pixmap));
        }
        self.lock.images = DecodedImages(decoded.images);
        lock.dirty = true;
        self.queue_redraw();
    }
}

/// The prepared sprite for `sprite`, if the last layout had it with the same
/// content, size and padding.
fn take_unchanged(
    old: &mut Vec<PreparedSprite>,
    sprite: &Sprite,
    pad: u32,
) -> Option<PreparedSprite> {
    let index = old.iter().position(|prepared| {
        prepared.id == sprite.id
            && prepared.pad == pad
            && prepared.content == sprite.content
            && prepared.rect.width == sprite.rect.width
            && prepared.rect.height == sprite.rect.height
    })?;
    let mut prepared = old.swap_remove(index);
    prepared.rect = sprite.rect;
    prepared.layer = sprite.layer;
    prepared.part = sprite.part;
    Some(prepared)
}

/// The prepared clock glyph `sprite` takes the place of with a different
/// character: the one that blurs away as it arrives.
fn take_replaced_glyph(old: &mut Vec<PreparedSprite>, sprite: &Sprite) -> Option<PreparedSprite> {
    let Content::Glyph(arriving) = &sprite.content else {
        return None;
    };
    let index = old.iter().position(|prepared| {
        prepared.id == sprite.id
            && matches!(&prepared.content, Content::Glyph(going) if going.ch != arriving.ch)
    })?;
    Some(old.swap_remove(index))
}

fn prepare(sprite: Sprite, pad: u32, pixmap: &Pixmap) -> PreparedSprite {
    PreparedSprite {
        id: sprite.id,
        layer: sprite.layer,
        part: sprite.part,
        rect: sprite.rect,
        pad,
        pixels: [pixmap.width(), pixmap.height()],
        content: sprite.content,
        buffer: buffer_of(pixmap),
        born: None,
        glass_id: Id::new(),
    }
}

pub(super) fn output_picture(pixmap: &Pixmap) -> OutputPicture {
    OutputPicture {
        buffer: buffer_of(pixmap),
        pixels: [pixmap.width(), pixmap.height()],
    }
}

/// tiny-skia's premultiplied R, G, B, A bytes are `Abgr8888` little-endian.
fn buffer_of(pixmap: &Pixmap) -> MemoryRenderBuffer {
    MemoryRenderBuffer::from_slice(
        pixmap.data(),
        Fourcc::Abgr8888,
        (pixmap.width() as i32, pixmap.height() as i32),
        1,
        Transform::Normal,
        None,
    )
}

#[derive(Default)]
struct Decoded {
    pictures: Vec<(OutputId, Pixmap)>,
    images: HashMap<AssetId, Pixmap>,
}

fn decode_all(
    background: Option<Source>,
    sizes: &[(OutputId, [u32; 2])],
    images: Vec<(AssetId, Arc<[u8]>)>,
) -> Decoded {
    Decoded {
        pictures: framed(background, sizes),
        images: images
            .into_iter()
            .filter_map(|(id, bytes)| Some((id, decode(&bytes).ok()?)))
            .collect(),
    }
}

/// `source` decoded and cut to cover each output.
fn framed(source: Option<Source>, sizes: &[(OutputId, [u32; 2])]) -> Vec<(OutputId, Pixmap)> {
    let Some(source) = source else {
        return Vec::new();
    };
    let Some(picture) = read_source(source).and_then(|bytes| decode(&bytes).ok()) else {
        tracing::warn!("the lock screen picture could not be read");
        return Vec::new();
    };
    sizes
        .iter()
        .filter_map(|(output, [w, h])| Some((*output, cover(&picture, *w, *h)?)))
        .collect()
}

fn read_source(source: Source) -> Option<Arc<[u8]>> {
    match source {
        Source::Asset(bytes) => Some(bytes),
        Source::Path(path) => read_capped(Path::new(&path)).ok().map(Arc::from),
    }
}

/// The file at `path`, refused past the size any lock-screen picture may be.
pub(super) fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(Limits::ASSET_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > Limits::ASSET_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::FileTooLarge,
            "larger than a lock-screen picture may be",
        ));
    }
    Ok(bytes)
}

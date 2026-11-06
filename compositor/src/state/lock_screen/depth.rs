//! The depth effect: the background's subject, cut out of it by
//! `crown-depthcut` and framed exactly as the background is, to stand in
//! front of the clock.
//!
//! Only a picture on disk has depth: the lock's own, or the desktop wallpaper
//! the live background shows. A picture without a mask yet is cut by the
//! helper, waited on off the compositor thread; the model never loads here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use crownos_config::schema::LockBackground;
use lockscreen::{
    image::{self, Mask, background_zoom, cover, darken, decode, subject},
    layout::{Part, Rect},
    lockscene::AssetId,
    tiny_skia::{Color, Pixmap},
};

use super::visuals::{output_picture, read_capped};
use crate::{
    shell::lock_screen::{OutputSubject, PreparedSprite},
    state::State,
    utils::id::OutputId,
};

/// How many times smaller than its output a subject's coverage is measured.
const PROBE_SHRINK: u32 = 8;

/// One output's subject, as cut off-thread.
struct Cut {
    output: OutputId,
    subject: Pixmap,
    probe: Pixmap,
}

impl State {
    /// Cuts the background's subject out for every output, off-thread, when
    /// the clock has depth.
    pub(super) fn load_lock_subject(&mut self) {
        let Some(picture) = self.depth_picture() else {
            return;
        };
        let mask = match depthcut::mask_path(&picture) {
            Ok(mask) => mask,
            Err(err) => {
                tracing::debug!(%err, "the lock screen background has no depth");
                return;
            }
        };
        if self.lock.uncuttable.contains(&mask) {
            return;
        }
        let sizes = self.lock_output_pixels();
        let dim = self.lock.scene.current().config.dim;
        let generation = self.lock.generation;
        self.common.tasks.run(
            async move {
                let cut = tokio::task::spawn_blocking(move || subjects(&picture, dim, &sizes))
                    .await
                    .unwrap_or_else(|err| Err(err.into()));
                (mask, cut)
            },
            move |state, (mask, cut)| state.lock_subjects_ready(generation, mask, cut),
        );
    }

    /// The picture file behind the lock, when its clock has depth.
    fn depth_picture(&self) -> Option<PathBuf> {
        let config = &self.lock.scene.current().config;
        if !config.clock.depth {
            return None;
        }
        let path = match &config.background {
            LockBackground::Image(src) if AssetId::from_src(src).is_none() => src,
            LockBackground::Desktop => &self.config.current.appearance.wallpaper,
            LockBackground::Image(_) | LockBackground::Token(_) => return None,
        };
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    fn lock_subjects_ready(&mut self, generation: u64, mask: PathBuf, cut: Result<Vec<Cut>>) {
        let cuts = match cut {
            Ok(cuts) => cuts,
            Err(err) => {
                if self.lock.uncuttable.insert(mask) {
                    tracing::warn!("the lock screen clock has no depth: {err:#}");
                }
                return;
            }
        };
        if generation != self.lock.generation {
            return;
        }
        let zoom = background_zoom(&self.lock.scene.current().config.background);
        let Some(lock) = self.shell.lock_screen.active_mut() else {
            return;
        };
        for cut in cuts {
            let visuals = lock.visuals.entry(cut.output).or_default();
            let mut subject = OutputSubject {
                picture: output_picture(&cut.subject),
                probe: cut.probe,
                hides_clock: false,
            };
            subject.hides_clock = hides_clock(&subject, &visuals.sprites, visuals.size, zoom);
            visuals.subject = Some(subject);
        }
        self.queue_redraw();
    }
}

/// Whether `subject` would hide too much of the clock `sprites` lay out on
/// an output `size` logical pixels big, the background zoomed `zoom` past
/// covering it, as ColorOS keeps the time legible.
pub(super) fn hides_clock(
    subject: &OutputSubject,
    sprites: &[PreparedSprite],
    size: [f32; 2],
    zoom: f32,
) -> bool {
    let Some([left, top, right, bottom]) = sprites
        .iter()
        .filter(|sprite| matches!(sprite.part, Some(Part::ClockGlyph(_))))
        .map(|sprite| sprite.rect)
        .fold(None, |bounds: Option<[f32; 4]>, rect| {
            let [left, top, right, bottom] =
                bounds.unwrap_or([f32::MAX, f32::MAX, f32::MIN, f32::MIN]);
            Some([
                left.min(rect.x),
                top.min(rect.y),
                right.max(rect.x + rect.width),
                bottom.max(rect.y + rect.height),
            ])
        })
    else {
        return false;
    };
    let clock = Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    };
    image::hides_clock(&subject.probe, clock, size, zoom)
}

/// `picture`'s subject covering each output, dimmed as the background is.
fn subjects(picture: &Path, dim: f32, sizes: &[(OutputId, [u32; 2])]) -> Result<Vec<Cut>> {
    let mask = Mask::decode(&read(&cut_mask(picture)?)?)?;
    let background = decode(&read(picture)?)?;
    Ok(sizes
        .iter()
        .filter_map(|(output, [width, height])| {
            let mut cut = subject(&cover(&background, *width, *height)?, &mask);
            darken(&mut cut, dim);
            Some(Cut {
                output: *output,
                subject: cut,
                probe: probe(&mask, [*width, *height])?,
            })
        })
        .collect())
}

/// The mask's coverage of a `pixels` output, framed as the subject is, at
/// [`PROBE_SHRINK`] of its size.
fn probe(mask: &Mask, [width, height]: [u32; 2]) -> Option<Pixmap> {
    let mut blank = Pixmap::new(width.div_ceil(PROBE_SHRINK), height.div_ceil(PROBE_SHRINK))?;
    blank.fill(Color::WHITE);
    Some(subject(&blank, mask))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    read_capped(path).with_context(|| format!("reading {}", path.display()))
}

/// The mask cut for `picture`, cutting it first if there is none yet.
fn cut_mask(picture: &Path) -> Result<PathBuf> {
    if let Some(mask) = depthcut::cached_mask(picture) {
        return Ok(mask);
    }
    let status = depthcut::spawn_cut(picture)?
        .wait()
        .context("waiting for the depth cut")?;
    if !status.success() {
        bail!("{} {status}", depthcut::HELPER_NAME);
    }
    depthcut::cached_mask(picture).context("the depth cut left no mask")
}

#[cfg(test)]
mod tests {
    use lockscreen::layout::{ClockGroup, Content, Layer};
    use smithay::backend::renderer::element::Id;

    use super::*;

    const OUTPUT: [u32; 2] = [400, 200];

    /// A subject covering the left half of the output.
    fn left_half() -> OutputSubject {
        let coverage = (0..OUTPUT[1])
            .flat_map(|_| (0..OUTPUT[0]).map(|column| if column < OUTPUT[0] / 2 { 255 } else { 0 }))
            .collect();
        let mask = Mask::new(OUTPUT[0], OUTPUT[1], coverage).expect("sized");
        OutputSubject {
            picture: output_picture(&Pixmap::new(OUTPUT[0], OUTPUT[1]).expect("size")),
            probe: probe(&mask, OUTPUT).expect("size"),
            hides_clock: false,
        }
    }

    fn glyph(rect: Rect) -> PreparedSprite {
        let pixmap = Pixmap::new(1, 1).expect("size");
        PreparedSprite {
            id: 0,
            layer: Layer::Clock,
            part: Some(Part::ClockGlyph(ClockGroup::Hours)),
            rect,
            pad: 0,
            pixels: [1, 1],
            content: Content::Divider {
                color: Default::default(),
            },
            buffer: output_picture(&pixmap).buffer,
            born: None,
            glass_id: Id::new(),
        }
    }

    #[test]
    fn a_subject_over_most_of_the_clock_gives_way() {
        let subject = left_half();
        let under = Rect {
            x: 20.0,
            y: 20.0,
            width: 60.0,
            height: 40.0,
        };
        let beside = Rect { x: 220.0, ..under };
        let size = OUTPUT.map(|side| side as f32);
        let doubled = size.map(|side| side / 2.0);
        assert!(hides_clock(&subject, &[glyph(under)], doubled, 1.0));
        assert!(!hides_clock(&subject, &[glyph(beside)], size, 1.0));
        assert!(!hides_clock(&subject, &[], size, 1.0));
    }
}

//! The built-in lock screen: whether it is up, what it is doing, and the
//! pixels prepared for each output.
//!
//! Up means the desktop is neither drawn nor typeable, from the instant the
//! lock is asked for until the unlock has finished animating. Only a password
//! the lock's own PAM check accepted brings it down.

use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use lockscreen::{
    clock::MORPH_DURATION,
    flow::LockFlow,
    layout::{Content, Layer, Part, Rect},
};
use smithay::{
    backend::renderer::{element::Id, element::memory::MemoryRenderBuffer},
    output::Output,
};

use crate::utils::id::OutputId;

/// One sprite of the layout, rasterised for one output.
#[derive(Debug)]
pub struct PreparedSprite {
    pub id: u64,
    pub layer: Layer,
    pub part: Option<Part>,
    pub rect: Rect,
    /// Transparent pixels around the content, for a blur to spread into.
    pub pad: u32,
    /// Size of the rasterised buffer, padding included.
    pub pixels: [u32; 2],
    pub content: Content,
    pub buffer: MemoryRenderBuffer,
    /// When this clock glyph replaced the one before it, while it blurs in.
    pub born: Option<Instant>,
    /// Names the sprite's glass, should it be drawn as glass, for its life.
    pub glass_id: Id,
}

/// What one output shows of the lock.
#[derive(Debug, Default)]
pub struct OutputVisuals {
    pub scale: f64,
    /// Logical size laid out for.
    pub size: [f32; 2],
    pub sprites: Vec<PreparedSprite>,
    /// Clock glyphs blurring away, and when they started to.
    pub leaving: Vec<(PreparedSprite, Instant)>,
    /// The background picture, covering the output pixel for pixel.
    pub picture: Option<MemoryRenderBuffer>,
    /// The background's subject, framed like the picture, to stand in front
    /// of the clock.
    pub depth: Option<MemoryRenderBuffer>,
}

impl OutputVisuals {
    pub fn is_morphing(&self, now: Instant) -> bool {
        let running = |since: Instant| now.saturating_duration_since(since) < MORPH_DURATION;
        self.leaving.iter().any(|(_, since)| running(*since))
            || self
                .sprites
                .iter()
                .any(|sprite| sprite.born.is_some_and(running))
    }

    /// Lets go of glyphs that have finished blurring away.
    pub fn settle_morphs(&mut self, now: Instant) {
        self.leaving
            .retain(|(_, since)| now.saturating_duration_since(*since) < MORPH_DURATION);
    }
}

/// Where the background comes from while locked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Backdrop {
    /// The wallpaper surfaces the desktop already has.
    Wallpaper,
    /// A picture, once it has been decoded.
    Picture,
    Color([f32; 4]),
}

pub struct ActiveLock {
    pub flow: LockFlow,
    pub backdrop: Backdrop,
    pub dim: f32,
    pub blur: f32,
    /// How far the clock's corners are pulled, as its settings last said.
    pub clock_warp: [[f32; 2]; 4],
    /// The clock's digits are frosted glass.
    pub clock_glass: bool,
    pub caps_lock: bool,
    pub visuals: HashMap<OutputId, OutputVisuals>,
    /// Layout or pixels are stale on every output.
    pub dirty: bool,
    blanked: HashSet<Output>,
    pub ids: LockIds,
}

/// Stable element ids, so the damage tracker sees the same backdrop, dim and
/// glass from one frame to the next.
#[derive(Debug)]
pub struct LockIds {
    pub base: Id,
    pub color: Id,
    pub dim: Id,
    pub glass: Id,
}

impl Default for LockIds {
    fn default() -> Self {
        Self {
            base: Id::new(),
            color: Id::new(),
            dim: Id::new(),
            glass: Id::new(),
        }
    }
}

#[derive(Default)]
pub struct LockScreen {
    active: Option<ActiveLock>,
}

impl LockScreen {
    /// Up, whether still animating in, showing, or animating out.
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.active.as_ref().is_some_and(|lock| {
            lock.flow.is_animating(now)
                || lock
                    .visuals
                    .values()
                    .any(|visuals| visuals.is_morphing(now))
        })
    }

    pub fn begin(&mut self, flow: LockFlow, backdrop: Backdrop, dim: f32, blur: f32) {
        self.active = Some(ActiveLock {
            flow,
            backdrop,
            dim,
            blur,
            clock_warp: [[0.0; 2]; 4],
            clock_glass: false,
            caps_lock: false,
            visuals: HashMap::new(),
            dirty: true,
            blanked: HashSet::new(),
            ids: LockIds::default(),
        });
    }

    pub fn end(&mut self) {
        self.active = None;
    }

    pub fn active(&self) -> Option<&ActiveLock> {
        self.active.as_ref()
    }

    pub fn active_mut(&mut self) -> Option<&mut ActiveLock> {
        self.active.as_mut()
    }

    /// Records that `output` has drawn a frame with the lock up.
    pub fn output_blanked(&mut self, output: &Output) {
        if let Some(lock) = &mut self.active {
            lock.blanked.insert(output.clone());
        }
    }

    /// Whether every one of `outputs` has drawn the lock since it went up.
    pub fn has_blanked<'a>(&self, mut outputs: impl Iterator<Item = &'a Output>) -> bool {
        self.active
            .as_ref()
            .is_some_and(|lock| outputs.all(|output| lock.blanked.contains(output)))
    }
}

//! Where everything on the lock screen sits.
//!
//! A theme's layout tree, or the stock layout built from the settings, is
//! laid out with flexbox and cut into [`Sprite`]s: small pictures, each on the
//! clock screen or the password prompt [`Layer`], that a host rasterises once
//! with [`paint`](crate::paint) and moves around for free.

mod block;
mod clock;
mod content;
mod live;
mod node;
mod outline;
mod stock;
mod text;

use lockscene::{AssetId, LockScene, Placeholder};
use tiny_skia::Pixmap;

use crate::{clock::ClockFace, palette::Palette, user::User};

pub use content::{Content, FontFamily, GlyphPaint, GlyphRun, TextRun};
pub(crate) use content::{SHADOW_DROP, SHADOW_SPREAD};
pub(crate) use text::Shaper;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    /// The clock screen: clock, date, plugin slots, theme decoration.
    Clock,
    /// The password prompt: user, field, messages.
    Prompt,
}

/// Which live part a sprite is, for the few a host treats specially.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    SecureField,
    /// One character of the clock, its face or its shadow.
    ClockGlyph(ClockGroup),
}

/// The parts of the clock that move on their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockGroup {
    Hours,
    Separator,
    Minutes,
}

/// Logical-pixel rectangle, output-local.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn centre(&self) -> [f32; 2] {
        [self.x + self.width / 2.0, self.y + self.height / 2.0]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sprite {
    /// Stable across layouts for the same place in the tree, so a host can
    /// keep a sprite's texture while its content stays equal.
    pub id: u64,
    pub layer: Layer,
    pub part: Option<Part>,
    pub rect: Rect,
    pub content: Content,
}

impl Sprite {
    /// Logical pixels the content draws past `rect`: the least padding a
    /// host should paint it with.
    pub fn reach(&self) -> f32 {
        match &self.content {
            Content::Glyph(run) => run.reach(),
            _ => 0.0,
        }
    }

    pub fn clock_group(&self) -> Option<ClockGroup> {
        match self.part {
            Some(Part::ClockGlyph(group)) => Some(group),
            _ => None,
        }
    }
}

/// The live state the scene's placeholders show.
#[derive(Debug, Clone, Copy)]
pub struct Live<'a> {
    pub face: &'a ClockFace,
    /// `None` hides the user.
    pub user: Option<&'a User>,
    pub password_chars: usize,
    pub checking: bool,
    /// The line under the field, and whether it reports an error.
    pub notice: Option<(&'a str, bool)>,
    pub caps_lock: bool,
}

/// Decoded assets, by id.
pub trait Images {
    fn get(&self, id: &AssetId) -> Option<&Pixmap>;
}

impl Images for () {
    fn get(&self, _: &AssetId) -> Option<&Pixmap> {
        None
    }
}

impl<S: std::hash::BuildHasher> Images for std::collections::HashMap<AssetId, Pixmap, S> {
    fn get(&self, id: &AssetId) -> Option<&Pixmap> {
        std::collections::HashMap::get(self, id)
    }
}

/// Everything the conversion from a scene needs at hand.
pub(crate) struct Context<'a> {
    pub scene: &'a LockScene,
    pub live: Live<'a>,
    pub palette: Palette<'a>,
    pub images: &'a dyn Images,
    pub family: &'a str,
    pub mono: &'a str,
    pub text_scale: f32,
}

impl Context<'_> {
    fn placeholder_layer(name: &str) -> Layer {
        match Placeholder::parse(name) {
            Some(part) if part.is_prompt() => Layer::Prompt,
            _ => Layer::Clock,
        }
    }
}

/// Lays `scene` out on an output of `size` logical pixels.
pub(crate) fn sprites(shaper: &mut Shaper, context: &Context<'_>, size: [f32; 2]) -> Vec<Sprite> {
    let root = match &context.scene.decoration {
        Some(tree) => node::layout_root(context, &tree.root),
        None => stock::layout(context, size),
    };
    block::place(shaper, root, size)
}

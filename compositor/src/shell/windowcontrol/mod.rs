//! One output's Alt+Tab switcher.
//!
//! The `windowcontrol` crate keys its strip by [`WindowId`] and knows nothing
//! about tiles; this is where the active workspace is fed into it and where the
//! geometry the renderer and the pointer need is laid out once per frame.

mod icons;

use std::collections::HashMap;

use smithay::{
    backend::renderer::element::{Id, memory::MemoryRenderBuffer},
    utils::{Buffer, Logical, Point, Rectangle, Size},
};

use spacecontrol::{animations::spring::SpringProfile, scene::Canvas};
use windowcontrol::{Direction, Metrics, Slot, Switcher, layout};

pub use icons::ICON_SIZE;

use crate::{shell::monitor::Monitor, utils::id::WindowId};

/// What the damage tracker knows a card's own quads by.
#[derive(Debug)]
pub struct CardIds {
    /// What a see-through window is laid on, so the card behind does not show
    /// through it.
    pub base: Id,
    pub shadow: Id,
}

impl Default for CardIds {
    fn default() -> Self {
        Self {
            base: Id::new(),
            shadow: Id::new(),
        }
    }
}

#[derive(Debug)]
pub struct WindowControl {
    switcher: Switcher<WindowId>,
    metrics: Metrics,
    frost: Id,
    cards: HashMap<WindowId, CardIds>,
    icons: icons::CardIcons,
    canvas: Option<Canvas>,
    slots: Vec<Slot>,
    /// Indices into `slots`, top card first.
    order: Vec<usize>,
    /// Width over height of every window seen since opening, so one that has
    /// closed keeps its shape while it collapses.
    aspects: HashMap<WindowId, f64>,
}

impl Default for WindowControl {
    fn default() -> Self {
        Self {
            switcher: Switcher::default(),
            metrics: Metrics::default(),
            frost: Id::new(),
            cards: HashMap::new(),
            icons: icons::CardIcons::default(),
            canvas: None,
            slots: Vec::new(),
            order: Vec::new(),
            aspects: HashMap::new(),
        }
    }
}

impl WindowControl {
    /// Comes up over `monitor`'s active workspace. Returns whether there was
    /// anything to show.
    pub fn open(&mut self, monitor: &Monitor, direction: Direction) -> bool {
        self.aspects.clear();
        self.cards.clear();
        self.icons.forget_windows();
        let opened = self
            .switcher
            .open(monitor.active().recent_windows(), direction);
        if opened {
            self.relayout(monitor);
        }
        opened
    }

    pub fn advance(&mut self, direction: Direction) {
        self.switcher.advance(direction);
    }

    pub fn commit(&mut self) -> Option<WindowId> {
        self.switcher.commit()
    }

    pub fn dismiss(&mut self) {
        self.switcher.dismiss();
    }

    pub fn scroll(&mut self, entries: f64) {
        self.switcher.scroll(entries);
    }

    pub fn release_scroll(&mut self) {
        self.switcher.release_scroll();
    }

    /// Lifts whatever is under `at`; true if that changed anything.
    pub fn motion(&mut self, at: Point<f64, Logical>) -> bool {
        self.switcher.hover(self.slot_at(at));
        !self.switcher.strip().at_rest()
    }

    /// Commits the card under `at`; anywhere else dismisses.
    pub fn click(&mut self, at: Point<f64, Logical>) -> Option<WindowId> {
        match self.slot_at(at) {
            Some(index) => self.switcher.commit_at(index),
            None => {
                self.switcher.dismiss();
                None
            }
        }
    }

    fn slot_at(&self, at: Point<f64, Logical>) -> Option<usize> {
        layout::slot_at(&self.slots, &self.order, at)
    }

    pub fn step(&mut self, dt: f32) {
        self.switcher.step(dt);
    }

    pub fn settle(&mut self) {
        self.switcher.settle();
    }

    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.switcher.set_profile(profile);
    }

    /// Follows windows opening and closing, then places everything for this
    /// frame.
    pub fn relayout(&mut self, monitor: &Monitor) {
        let workspace = monitor.active();
        if self.switcher.is_open() {
            self.switcher.sync(workspace.recent_windows());
        }
        for tile in workspace.tiles() {
            let size = tile.target().size;
            if size.w > 0 && size.h > 0 {
                self.aspects
                    .insert(tile.id(), f64::from(size.w) / f64::from(size.h));
            }
        }
        let entries = self.switcher.strip().entries();
        for entry in entries {
            self.cards.entry(entry.key).or_default();
        }
        self.icons.update(
            workspace,
            entries.iter().map(|entry| entry.key),
            monitor.output().current_scale().fractional_scale(),
        );

        let canvas = Canvas::new(
            Rectangle::from_size(monitor.geometry().size),
            monitor.usable(),
        );
        self.canvas = Some(canvas);
        let aspects = &self.aspects;
        layout::arrange(
            canvas,
            &self.metrics,
            self.switcher.strip(),
            |id| aspects.get(&id).copied().unwrap_or(16.0 / 10.0),
            &mut self.slots,
        );
        layout::front_to_back(&self.slots, &mut self.order);
    }

    pub fn is_open(&self) -> bool {
        self.switcher.is_open()
    }

    pub fn is_visible(&self) -> bool {
        self.switcher.is_visible()
    }

    pub fn is_active(&self) -> bool {
        self.switcher.is_active()
    }

    pub fn switcher(&self) -> &Switcher<WindowId> {
        &self.switcher
    }

    pub fn frost_id(&self) -> &Id {
        &self.frost
    }

    pub fn card_ids(&self, window: WindowId) -> Option<&CardIds> {
        self.cards.get(&window)
    }

    /// `window`'s app icon and its size in pixels.
    pub fn icon(&self, window: WindowId) -> Option<(&MemoryRenderBuffer, Size<i32, Buffer>)> {
        self.icons.get(window)
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// Indices into [`Self::slots`], top card first.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    /// How far `window` has come out of the stack to the top, 0 to 1.
    pub fn focus_of(&self, window: WindowId) -> f64 {
        self.switcher
            .strip()
            .entries()
            .iter()
            .find(|entry| entry.key == window)
            .map_or(0.0, |entry| entry.focus())
    }

    /// The point the stack grows out of as it comes up.
    pub fn centre(&self) -> Point<f64, Logical> {
        self.canvas.map_or_else(Point::default, |canvas| {
            spacecontrol::scene::centre(canvas.usable.to_f64())
        })
    }
}

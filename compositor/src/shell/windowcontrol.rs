//! One output's Alt+Tab switcher.
//!
//! The `windowcontrol` crate keys its strip by [`WindowId`] and knows nothing
//! about tiles; this is where the active workspace is fed into it and where the
//! geometry the renderer and the pointer need is laid out once per frame.

use std::collections::HashMap;

use smithay::{
    backend::renderer::element::Id,
    utils::{Logical, Point, Rectangle},
};

use spacecontrol::{animations::spring::SpringProfile, scene::Canvas};
use windowcontrol::{Direction, Metrics, Slot, Switcher, layout};

use crate::{shell::monitor::Monitor, utils::id::WindowId};

/// What the damage tracker knows the strip's own quads by.
#[derive(Debug)]
pub struct WindowControlIds {
    pub panel: Id,
    pub shadow: Id,
    pub ring: Id,
}

impl Default for WindowControlIds {
    fn default() -> Self {
        Self {
            panel: Id::new(),
            shadow: Id::new(),
            ring: Id::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct WindowControl {
    switcher: Switcher<WindowId>,
    metrics: Metrics,
    ids: WindowControlIds,
    canvas: Option<Canvas>,
    panel: Rectangle<f64, Logical>,
    slots: Vec<Slot>,
    /// Width over height of every window seen since opening, so one that has
    /// closed keeps its shape while it collapses.
    aspects: HashMap<WindowId, f64>,
}

impl WindowControl {
    /// Rises over `monitor`'s active workspace. Returns whether there was
    /// anything to show.
    pub fn open(&mut self, monitor: &Monitor, direction: Direction) -> bool {
        self.aspects.clear();
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

    pub fn click(&mut self, at: Point<f64, Logical>) -> Option<WindowId> {
        match self.slot_at(at) {
            Some(index) => self.switcher.commit_at(index),
            None => None,
        }
    }

    fn slot_at(&self, at: Point<f64, Logical>) -> Option<usize> {
        layout::slot_at(self.panel, &self.slots, at)
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

        let canvas = Canvas::new(
            Rectangle::from_size(monitor.geometry().size),
            monitor.usable(),
        );
        self.canvas = Some(canvas);
        let aspects = &self.aspects;
        self.panel = layout::arrange(
            canvas,
            &self.metrics,
            self.switcher.progress(),
            self.switcher.strip(),
            |id| aspects.get(&id).copied().unwrap_or(16.0 / 10.0),
            &mut self.slots,
        );
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

    pub fn ids(&self) -> &WindowControlIds {
        &self.ids
    }

    pub fn panel(&self) -> Rectangle<f64, Logical> {
        self.panel
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// The thumbnail `window` is shown as, if it is in the strip.
    pub fn slot_of(&self, window: WindowId) -> Option<Slot> {
        let index = self
            .switcher
            .strip()
            .entries()
            .iter()
            .position(|entry| entry.key == window)?;
        self.slots.get(index).copied()
    }

    /// The point the workspace behind the strip steps back towards.
    pub fn vanishing_point(&self) -> Point<f64, Logical> {
        self.canvas.map_or_else(Point::default, |canvas| {
            spacecontrol::scene::centre(canvas.usable.to_f64())
        })
    }
}

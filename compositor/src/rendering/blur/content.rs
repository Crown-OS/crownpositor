//! What actually changed under the glass this frame.
//!
//! A backdrop owes the band around changed content, because blur spreads a
//! change past the damage that carried it. The output's damage cannot say what
//! changed: it also holds the bands themselves, and smithay shapes it into
//! tiles, so a repainted band comes back larger than it was offered. Owing a
//! band for that excess hands bands back and forth between overlapping glass
//! forever. A second tracker over every element *except* the glass answers the
//! question directly.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use smithay::{
    backend::renderer::{
        damage::OutputDamageTracker,
        element::{Element, Id},
    },
    utils::{Physical, Point, Rectangle},
};

use crate::utils::region;

#[derive(Debug, Default)]
struct State {
    tracker: Option<OutputDamageTracker>,
    glass: HashSet<Id>,
    /// `None` until observed: a frame nobody observed treats all damage as
    /// changed, which is always safe.
    changed: Option<Vec<Rectangle<i32, Physical>>>,
}

/// A shared handle; cloning is cheap and every clone sees the same frame.
#[derive(Debug, Clone, Default)]
pub struct ContentDamage(Rc<RefCell<State>>);

impl ContentDamage {
    pub(super) fn begin_frame(&self) {
        let mut state = self.0.borrow_mut();
        state.glass.clear();
        state.changed = None;
    }

    pub(super) fn add_glass(&self, id: &Id) {
        self.0.borrow_mut().glass.insert(id.clone());
    }

    /// Runs the content tracker over this frame's elements, once they are
    /// built and before they are drawn. A frame with no glass skips it.
    pub fn observe<E: Element>(
        &self,
        tracker: impl FnOnce() -> OutputDamageTracker,
        elements: &[E],
    ) {
        let mut state = self.0.borrow_mut();
        if state.glass.is_empty() {
            return;
        }
        let State {
            tracker: slot,
            glass,
            changed,
        } = &mut *state;
        let content: Vec<&E> = elements
            .iter()
            .filter(|element| !glass.contains(element.id()))
            .collect();
        let tracker = slot.get_or_insert_with(tracker);
        *changed = tracker
            .damage_output(1, &content)
            .ok()
            .map(|(damage, _)| damage.cloned().unwrap_or_default());
    }

    /// The part of `damage` where content really changed, both in coordinates
    /// relative to `origin`.
    pub(super) fn changed(
        &self,
        damage: &[Rectangle<i32, Physical>],
        origin: Point<i32, Physical>,
    ) -> Vec<Rectangle<i32, Physical>> {
        match &self.0.borrow().changed {
            Some(changed) => region::intersect(
                damage,
                changed
                    .iter()
                    .map(|rect| Rectangle::new(rect.loc - origin, rect.size)),
            ),
            None => damage.to_vec(),
        }
    }
}

//! What actually changed under the glass this frame.
//!
//! A blur spreads a changed pixel past the rectangle it arrived in, so a
//! backdrop has to repaint a band around every change beneath it. The output's
//! damage cannot say what changed: it holds those bands themselves, and
//! smithay shapes it into tiles, so a repainted band would come back larger
//! than it was asked for and overlapping glass would hand bands back and forth
//! forever. A second tracker over every element *except* the glass answers
//! the question directly — before the frame is drawn, so each backdrop can
//! claim its band as this frame's damage rather than owing it to the next.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use smithay::{
    backend::renderer::{
        damage::OutputDamageTracker,
        element::{Element, Id},
    },
    utils::{Physical, Rectangle},
};

use crate::rendering::blur::scene::grow;

#[derive(Debug, Default)]
struct State {
    tracker: Option<OutputDamageTracker>,
    glass: HashSet<Id>,
    /// `None` until observed: a frame nobody observed treats every pixel as
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

    /// Where a change within `reach` of `geometry` lands on it, relative to
    /// `geometry`. `None` when the frame was never observed.
    pub(super) fn changed_near(
        &self,
        geometry: Rectangle<i32, Physical>,
        reach: i32,
    ) -> Option<Vec<Rectangle<i32, Physical>>> {
        let state = self.0.borrow();
        let changed = state.changed.as_ref()?;
        Some(
            changed
                .iter()
                .filter_map(|rect| grow(*rect, reach).intersection(geometry))
                .map(|rect| Rectangle::new(rect.loc - geometry.loc, rect.size))
                .collect(),
        )
    }
}

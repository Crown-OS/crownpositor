//! Who owns each slot of a session's buffer ring.
//!
//! The whole pacing model of `crownos_screencast_v1` is this state machine: the
//! compositor renders only into slots it owns, hands each finished frame to
//! the client, and gets the slot back only through `release` — after the
//! client's release fence, when it gave one. Kept free of any protocol or
//! renderer type so every transition is testable without a display.

/// Slots per session, as the protocol's `ring.max_slots` fixes it.
pub const MAX_SLOTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotOwner {
    /// Free for the compositor to render into.
    Compositor,
    /// A render into the slot is in flight and its frame not yet sent.
    Rendering,
    /// Handed out by a `frame` event and not released since.
    Client,
    /// Released with a release point the compositor is still waiting on.
    Fenced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RingError {
    #[error("slot index {0} is not below {MAX_SLOTS}")]
    InvalidIndex(u32),
    #[error("slot {0} already holds a buffer")]
    AlreadyAttached(usize),
    #[error("slot {0} is not owned by the client")]
    NotClientOwned(usize),
}

/// What a `release` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Released {
    /// The slot is the compositor's again.
    Immediately,
    /// The slot waits for its release point.
    AfterFence,
    /// The slot was emptied by a constraints change; nothing happened.
    Ignored,
}

#[derive(Debug)]
struct Slot<B> {
    buffer: B,
    owner: SlotOwner,
}

#[derive(Debug)]
pub struct Ring<B> {
    slots: [Option<Slot<B>>; MAX_SLOTS],
}

impl<B> Default for Ring<B> {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}

impl<B: Clone> Ring<B> {
    pub fn index(raw: u32) -> Result<usize, RingError> {
        usize::try_from(raw)
            .ok()
            .filter(|index| *index < MAX_SLOTS)
            .ok_or(RingError::InvalidIndex(raw))
    }

    pub fn attach(&mut self, index: usize, buffer: B) -> Result<(), RingError> {
        let slot = &mut self.slots[index];
        if slot.is_some() {
            return Err(RingError::AlreadyAttached(index));
        }
        *slot = Some(Slot {
            buffer,
            owner: SlotOwner::Compositor,
        });
        Ok(())
    }

    /// Claims the first compositor-owned slot for a render.
    pub fn acquire(&mut self) -> Option<(usize, B)> {
        self.slots.iter_mut().enumerate().find_map(|(index, slot)| {
            let slot = slot
                .as_mut()
                .filter(|slot| slot.owner == SlotOwner::Compositor)?;
            slot.owner = SlotOwner::Rendering;
            Some((index, slot.buffer.clone()))
        })
    }

    /// Returns a claimed slot unused, when the render produced no frame.
    pub fn abandon(&mut self, index: usize) {
        self.transition(index, SlotOwner::Rendering, SlotOwner::Compositor);
    }

    /// Gives a rendered slot to the client. `false` if it was emptied while
    /// the render was in flight, in which case no frame may be sent for it.
    pub fn hand_out(&mut self, index: usize) -> bool {
        self.transition(index, SlotOwner::Rendering, SlotOwner::Client)
    }

    pub fn release(&mut self, index: usize, fenced: bool) -> Result<Released, RingError> {
        let Some(slot) = self.slots[index].as_mut() else {
            return Ok(Released::Ignored);
        };
        if slot.owner != SlotOwner::Client {
            return Err(RingError::NotClientOwned(index));
        }
        if fenced {
            slot.owner = SlotOwner::Fenced;
            Ok(Released::AfterFence)
        } else {
            slot.owner = SlotOwner::Compositor;
            Ok(Released::Immediately)
        }
    }

    pub fn fence_signalled(&mut self, index: usize) -> bool {
        self.transition(index, SlotOwner::Fenced, SlotOwner::Compositor)
    }

    /// Empties every slot, as a constraints change after `start` does.
    pub fn clear(&mut self) {
        self.slots = std::array::from_fn(|_| None);
    }

    #[cfg(test)]
    fn owner(&self, index: usize) -> Option<SlotOwner> {
        self.slots.get(index)?.as_ref().map(|slot| slot.owner)
    }

    #[cfg(test)]
    fn buffers(&self) -> impl Iterator<Item = &B> {
        self.slots.iter().flatten().map(|slot| &slot.buffer)
    }

    fn transition(&mut self, index: usize, from: SlotOwner, to: SlotOwner) -> bool {
        match self.slots.get_mut(index).and_then(Option::as_mut) {
            Some(slot) if slot.owner == from => {
                slot.owner = to;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_of(count: usize) -> Ring<u32> {
        let mut ring = Ring::default();
        for index in 0..count {
            assert_eq!(ring.attach(index, index as u32 * 10), Ok(()));
        }
        ring
    }

    #[test]
    fn indices_past_the_ring_are_refused() {
        assert_eq!(Ring::<u32>::index(3), Ok(3));
        assert_eq!(Ring::<u32>::index(4), Err(RingError::InvalidIndex(4)));
        assert_eq!(
            Ring::<u32>::index(u32::MAX),
            Err(RingError::InvalidIndex(u32::MAX))
        );
    }

    #[test]
    fn a_slot_is_attached_once() {
        let mut ring = ring_of(1);
        assert_eq!(ring.attach(0, 7), Err(RingError::AlreadyAttached(0)));
    }

    #[test]
    fn frames_go_round_the_ring_and_stop_when_the_client_holds_everything() {
        let mut ring = ring_of(2);

        let (first, buffer) = ring.acquire().expect("a free slot");
        assert_eq!((first, buffer), (0, 0));
        assert!(ring.hand_out(first));

        let (second, _) = ring.acquire().expect("the other slot");
        assert_eq!(second, 1);
        assert!(ring.hand_out(second));

        // The encoder is behind: nothing to render into, so nothing is queued.
        assert_eq!(ring.acquire(), None);

        assert_eq!(ring.release(first, false), Ok(Released::Immediately));
        assert_eq!(ring.acquire().map(|(index, _)| index), Some(first));
    }

    #[test]
    fn an_abandoned_render_frees_the_slot_without_a_frame() {
        let mut ring = ring_of(1);
        let (index, _) = ring.acquire().expect("a free slot");
        ring.abandon(index);
        assert_eq!(ring.owner(index), Some(SlotOwner::Compositor));
    }

    #[test]
    fn only_client_owned_slots_can_be_released() {
        let mut ring = ring_of(1);
        assert_eq!(ring.release(0, false), Err(RingError::NotClientOwned(0)));

        let (index, _) = ring.acquire().expect("a free slot");
        assert_eq!(
            ring.release(index, false),
            Err(RingError::NotClientOwned(0))
        );
    }

    #[test]
    fn a_fenced_release_waits_for_its_point() {
        let mut ring = ring_of(1);
        let (index, _) = ring.acquire().expect("a free slot");
        ring.hand_out(index);

        assert_eq!(ring.release(index, true), Ok(Released::AfterFence));
        assert_eq!(ring.acquire(), None, "not before the fence");

        assert!(ring.fence_signalled(index));
        assert!(ring.acquire().is_some());
    }

    #[test]
    fn a_constraints_change_empties_the_ring_and_forgives_stale_releases() {
        let mut ring = ring_of(2);
        let (index, _) = ring.acquire().expect("a free slot");
        ring.hand_out(index);

        ring.clear();
        assert_eq!(ring.buffers().count(), 0);
        assert_eq!(ring.release(index, false), Ok(Released::Ignored));
        assert!(
            !ring.hand_out(index),
            "a render into an emptied slot is dropped"
        );
        assert_eq!(ring.attach(index, 99), Ok(()));
    }
}

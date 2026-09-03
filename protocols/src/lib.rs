//! Wayland protocols smithay does not ship (yet).
//!
//! Each module here is written the way smithay writes its own: a state type
//! whose `Dispatch` impls are generic over the compositor data `D`, so the
//! protocol logic stays decoupled from the compositor entirely — the
//! compositor owns the delegation glue, and its renderer only ever sees the
//! cached state left behind on surfaces.
//!
//! That decoupling is why these live in their own crate: nothing in here may
//! reach back into the compositor, and the crate boundary is what enforces it.

pub mod background_effect;
pub mod region;

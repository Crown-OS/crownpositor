//! Wayland protocols smithay does not ship (yet).
//!
//! Each module here is written the way smithay writes its own: a state type
//! whose `Dispatch` impls are generic over the compositor data `D`, so the
//! protocol logic stays decoupled from [`State`] — the `handlers` module owns
//! the delegation glue, and the renderer only ever sees the cached state left
//! behind on surfaces.
//!
//! [`State`]: crate::state::State

pub mod background_effect;

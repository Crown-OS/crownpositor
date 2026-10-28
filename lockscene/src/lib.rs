//! The lock screen as data: what `crownplugind` composes and sends over
//! `crownos_plugin_host_v1`, what the compositor draws, and the bounds both
//! sides hold it to.
//!
//! Nothing here can unlock anything. A scene is display-only: trees carry no
//! interactive nodes, images are content-addressed assets, and every size is
//! capped so a hostile scene costs bounded memory and time to draw.

mod asset;
mod compose;
mod error;
mod limits;
mod placeholder;
mod scene;
mod validate;

pub use asset::{ASSET_SRC_PREFIX, AssetId};
pub use compose::compose;
pub use error::SceneError;
pub use limits::Limits;
pub use placeholder::Placeholder;
pub use scene::{LockScene, Slot};
pub use validate::{ValidScene, validate, validate_slot};

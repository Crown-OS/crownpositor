//! What the compositor draws around a window.
//!
//! Only floating windows are framed — a tiled one fills the slot the layout
//! gave it, and a titlebar there would be a row of pixels the user cannot use
//! for anything. The predicate lives on the tile itself
//! ([`Tile::is_decorated`]), and everything here just draws what it decides.
//!
//! [`Tile::is_decorated`]: crate::shell::tile::Tile::is_decorated

pub mod label;
pub mod palette;
pub mod shadow;
pub mod title_bar;
pub mod window;

pub use label::TextRenderer;
pub use palette::FramePalette;
pub use shadow::GlassShadow;
pub use title_bar::{TitleBar, TitleBarParams};
pub use window::{Border, WindowDecoration};

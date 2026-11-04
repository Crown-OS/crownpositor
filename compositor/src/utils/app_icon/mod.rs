//! An application's icon, found the way a launcher finds it: the desktop entry
//! named after its app id says which icon, and the icon theme says where it is.

mod desktop_entry;
mod theme;

use std::{fs, path::PathBuf};

use crate::utils::raster::{self, FilterQuality, Pixmap};

/// `app_id`'s icon, `side` pixels square. `None` when no entry or theme has
/// one, or the file does not decode.
pub fn rasterise(app_id: &str, side: u32) -> Option<Pixmap> {
    let name = desktop_entry::icon_name(app_id).unwrap_or_else(|| app_id.to_lowercase());
    let path = match PathBuf::from(&name) {
        path if path.is_absolute() => path,
        _ => theme::find(&name)?,
    };
    let data = fs::read(&path).ok()?;
    let vector = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
    match vector {
        true => raster::vector(&data, side),
        false => raster::bitmap(&data, side, FilterQuality::Bicubic),
    }
}

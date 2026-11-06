use std::path::Path;

use anyhow::Result;
use image::{ImageReader, RgbImage};

use crate::util::fit::within_long_side;

/// The picture at `path`, shrunk straight after decoding to at most
/// `long_side` on its longer side: the full-size pixels are gone before the
/// model loads. No EXIF rotation, as the lock screen draws it unrotated.
pub fn load(path: &Path, long_side: u32) -> Result<RgbImage> {
    let decoded = ImageReader::open(path)?.with_guessed_format()?.decode()?;
    let (width, height) = within_long_side((decoded.width(), decoded.height()), long_side);
    Ok(decoded.thumbnail_exact(width, height).into_rgb8())
}

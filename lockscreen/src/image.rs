//! Pictures the lock screen shows: decoded with hard limits, so a hostile or
//! merely huge file costs bounded memory, and premultiplied for compositing.

use std::io::Cursor;

use image::{ImageReader, Limits as DecodeLimits};
use lockscene::Limits;
use resvg::usvg;
use thiserror::Error;
use tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};

use crate::util::blur::box_blur;

/// Longest side a vector picture is rasterised at.
const SVG_SIDE: f32 = 3840.0;
/// The blurred backdrop is drawn at this fraction of the output and scaled up
/// on the GPU: a blur has no detail to lose.
const BACKDROP_DOWNSCALE: u32 = 8;
/// Blur radius, in downscaled pixels, at full `lock.blur`.
const BACKDROP_RADIUS: f32 = 6.0;

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("the picture is larger than {} pixels", Limits::IMAGE_PIXELS)]
    TooLarge,
    #[error(transparent)]
    Decode(#[from] image::ImageError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the vector picture did not parse: {0}")]
    Svg(#[from] usvg::Error),
    #[error("the picture is empty")]
    Empty,
}

pub fn decode(bytes: &[u8]) -> Result<Pixmap, ImageError> {
    if looks_like_svg(bytes) {
        decode_svg(bytes)
    } else {
        decode_raster(bytes)
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    let head = String::from_utf8_lossy(head);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

fn decode_raster(bytes: &[u8]) -> Result<Pixmap, ImageError> {
    let (width, height) = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    if u64::from(width) * u64::from(height) > Limits::IMAGE_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = DecodeLimits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(Limits::IMAGE_PIXELS * 4 * 2);
    reader.limits(limits);
    let rgba = reader.decode()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut data = rgba.into_raw();
    for pixel in data.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    let size = IntSize::from_wh(width, height).ok_or(ImageError::Empty)?;
    Pixmap::from_vec(data, size).ok_or(ImageError::Empty)
}

fn decode_svg(bytes: &[u8]) -> Result<Pixmap, ImageError> {
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options)?;
    let size = tree.size();
    let scale = SVG_SIDE / size.width().max(size.height());
    let width = (size.width() * scale).round() as u32;
    let height = (size.height() * scale).round() as u32;
    let mut pixmap = Pixmap::new(width.max(1), height.max(1)).ok_or(ImageError::Empty)?;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// `source` scaled to cover `width` × `height` and cropped to it, centred.
pub fn cover(source: &Pixmap, width: u32, height: u32) -> Option<Pixmap> {
    let mut target = Pixmap::new(width.max(1), height.max(1))?;
    let scale = (target.width() as f32 / source.width() as f32)
        .max(target.height() as f32 / source.height() as f32);
    let dx = (target.width() as f32 - source.width() as f32 * scale) / 2.0;
    let dy = (target.height() as f32 - source.height() as f32 * scale) / 2.0;
    let paint = PixmapPaint {
        quality: FilterQuality::Bicubic,
        ..PixmapPaint::default()
    };
    target.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &paint,
        Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
        None,
    );
    Some(target)
}

/// Blurs premultiplied `pixmap` in place by about `radius` pixels.
pub fn blur(pixmap: &mut Pixmap, radius: usize) {
    box_blur(pixmap, radius);
}

/// A small, blurred copy of `source` covering `width` × `height` once scaled
/// up, or `None` when `strength` asks for no blur.
pub fn backdrop(source: &Pixmap, width: u32, height: u32, strength: f32) -> Option<Pixmap> {
    let radius = (BACKDROP_RADIUS * strength.clamp(0.0, 1.0)).round() as usize;
    if radius == 0 {
        return None;
    }
    let mut small = cover(
        source,
        (width / BACKDROP_DOWNSCALE).max(1),
        (height / BACKDROP_DOWNSCALE).max(1),
    )?;
    box_blur(&mut small, radius);
    Some(small)
}

/// Darkens premultiplied `pixmap` as a black wash at `amount` over it would,
/// leaving its alpha alone.
pub fn darken(pixmap: &mut Pixmap, amount: f32) {
    let keep = (1.0 - amount.clamp(0.0, 1.0)) * 255.0;
    let keep = keep.round() as u32;
    for pixel in pixmap.data_mut().chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * keep + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 128]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("encodes");
        bytes
    }

    #[test]
    fn rasters_decode_premultiplied() {
        let pixmap = decode(&png(4, 2)).expect("decodes");
        assert_eq!((pixmap.width(), pixmap.height()), (4, 2));
        let pixel = pixmap.pixels()[0];
        assert_eq!((pixel.red(), pixel.alpha()), (128, 128));
    }

    #[test]
    fn a_header_claiming_too_many_pixels_is_refused_before_decoding() {
        let mut bytes = png(1, 1);
        // IHDR width and height, big-endian, after the 8-byte signature and
        // the chunk length and type.
        bytes[16..20].copy_from_slice(&60_000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&60_000u32.to_be_bytes());
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        assert!(matches!(decode(&bytes), Err(ImageError::TooLarge)));
    }

    fn crc32(bytes: &[u8]) -> u32 {
        !bytes.iter().fold(!0u32, |crc, &byte| {
            (0..8).fold(crc ^ u32::from(byte), |crc, _| {
                if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                }
            })
        })
    }

    #[test]
    fn vectors_render_and_never_load_files() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20" height="10">
            <rect width="20" height="10" fill="blue"/>
            <image xlink:href="/etc/passwd" width="20" height="10"/>
        </svg>"#;
        let pixmap = decode(svg).expect("renders");
        assert_eq!(pixmap.width(), SVG_SIDE as u32);
        assert_eq!(pixmap.pixels()[0].blue(), 255);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode(b"not really a png").is_err());
        assert!(decode(b"<svg").is_err());
    }

    #[test]
    fn covering_fills_the_target() {
        let source = decode(&png(40, 10)).expect("decodes");
        let covered = cover(&source, 10, 10).expect("fits");
        assert!(covered.pixels().iter().all(|pixel| pixel.alpha() > 0));
        assert!(backdrop(&source, 80, 80, 0.0).is_none());
        let blurred = backdrop(&source, 80, 80, 1.0).expect("blurs");
        assert_eq!((blurred.width(), blurred.height()), (10, 10));
    }

    #[test]
    fn darkening_keeps_alpha() {
        let mut pixmap = Pixmap::new(1, 1).expect("size");
        pixmap.fill(tiny_skia::Color::from_rgba8(200, 100, 50, 255));
        darken(&mut pixmap, 0.5);
        assert_eq!(pixmap.data(), &[100, 50, 25, 255]);
    }
}

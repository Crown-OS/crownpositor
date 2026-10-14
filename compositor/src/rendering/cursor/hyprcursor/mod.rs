//! Hyprcursor themes: per-shape zip archives of SVG or PNG frames, described
//! by hyprlang or TOML metadata. Read natively, so vector shapes rasterise at
//! exactly the pixel size every output scale needs.

mod meta;
mod raster;
mod theme;

use smithay::{backend::allocator::Fourcc, input::pointer::CursorIcon};

use crate::rendering::cursor::{
    hyprcursor::theme::{ShapeArchive, Theme},
    source::{CursorSource, RawFrame, RawImage, RawShape, shape_names},
};

pub struct HyprcursorSource {
    theme: Theme,
}

impl HyprcursorSource {
    /// `configured` wins, otherwise `HYPRCURSOR_THEME`. `None` when neither
    /// names an installed hyprcursor theme, which leaves the XCursor theme of
    /// the same name in charge.
    pub fn new(configured: Option<&str>) -> Option<Self> {
        let name = configured.map(str::to_owned).or_else(|| {
            std::env::var("HYPRCURSOR_THEME")
                .ok()
                .filter(|name| !name.is_empty())
        })?;
        let theme = Theme::find(&name);
        if theme.is_none() {
            tracing::debug!(name, "no hyprcursor theme by this name");
        }
        theme.map(|theme| Self { theme })
    }

    fn rasterise(archive: &mut ShapeArchive, size: u32) -> Option<RawShape> {
        let meta = archive.meta()?;
        let pick = meta.pick(size)?;
        let side = pick.side;
        let hotspot = (
            (meta.hotspot.0 * f64::from(side)).round(),
            (meta.hotspot.1 * f64::from(side)).round(),
        );

        let frames = pick
            .frames
            .iter()
            .map(|entry| {
                let data = archive.read(&entry.file)?;
                let pixmap = if entry.is_vector() {
                    raster::vector(&data, side)?
                } else {
                    raster::bitmap(&data, side, meta.resize)?
                };
                Some(RawFrame {
                    image: RawImage {
                        width: pixmap.width() as i32,
                        height: pixmap.height() as i32,
                        // Premultiplied `[r, g, b, a]` bytes: `0xAABBGGRR` as a
                        // little-endian word.
                        pixels: pixmap.take(),
                        format: Fourcc::Abgr8888,
                        hotspot,
                    },
                    delay: entry.delay,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        RawShape::new(frames)
    }
}

impl CursorSource for HyprcursorSource {
    fn describe(&self) -> String {
        format!(
            "hyprcursor theme '{}' at {}",
            self.theme.name,
            self.theme.root.display()
        )
    }

    fn shape(&self, icon: CursorIcon, size: u32) -> Option<RawShape> {
        let path = shape_names(icon).find_map(|name| self.theme.archive_for(name))?;
        let shape =
            ShapeArchive::open(&path).and_then(|mut archive| Self::rasterise(&mut archive, size));
        if shape.is_none() {
            tracing::warn!(path = %path.display(), "malformed hyprcursor shape, skipping");
        }
        shape
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write, time::Duration};

    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    const FRAME: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#000"/></svg>"##;

    fn write_shape(dir: &std::path::Path, name: &str, meta: &str, files: &[&str]) {
        let file = std::fs::File::create(dir.join(format!("{name}.hlc"))).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("meta.hl", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(meta.as_bytes()).unwrap();
        for frame in files {
            zip.start_file(*frame, SimpleFileOptions::default())
                .unwrap();
            zip.write_all(FRAME.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn theme_in(root: &std::path::Path) -> HyprcursorSource {
        let cursors = root.join("shapes");
        std::fs::create_dir_all(&cursors).unwrap();
        std::fs::write(
            root.join("manifest.hl"),
            "name = test\ncursors_directory = shapes\n",
        )
        .unwrap();
        write_shape(
            &cursors,
            "left_ptr",
            "hotspot_x = 0.25\nhotspot_y = 0.5\ndefine_override = default\ndefine_size = 0, a.svg",
            &["a.svg"],
        );
        write_shape(
            &cursors,
            "watch",
            "define_override = wait\ndefine_size = 0, 1.svg, 50\ndefine_size = 0, 2.svg, 50",
            &["1.svg", "2.svg"],
        );
        let theme = Theme::load(root).unwrap();
        HyprcursorSource { theme }
    }

    #[test]
    fn overrides_resolve_and_hotspots_scale_with_the_side() {
        let dir = tempdir();
        let source = theme_in(&dir);
        let shape = source.shape(CursorIcon::Default, 32).unwrap();
        let image = &shape.frames()[0].image;
        assert_eq!((image.width, image.height), (32, 32));
        assert_eq!(image.hotspot, (8.0, 16.0));
    }

    #[test]
    fn animated_shapes_keep_their_frames_and_delays() {
        let dir = tempdir();
        let source = theme_in(&dir);
        let shape = source.shape(CursorIcon::Wait, 24).unwrap();
        assert_eq!(shape.frames().len(), 2);
        assert_eq!(shape.frames()[1].delay, Duration::from_millis(50));
    }

    #[test]
    fn missing_shapes_defer_to_the_next_source() {
        let dir = tempdir();
        let source = theme_in(&dir);
        assert!(source.shape(CursorIcon::Crosshair, 24).is_none());
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crown-hyprcursor-{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("t")
                .replace("::", "-")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

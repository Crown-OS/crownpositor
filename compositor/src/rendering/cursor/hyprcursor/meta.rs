//! The two text files of a hyprcursor theme, the theme `manifest` and each
//! shape's `meta`, in either spelling the format allows: hyprlang (`.hl`) or
//! TOML (`.toml`).

use std::time::Duration;

/// `(key, value)` per entry. Hyprlang repeats a key once per entry, TOML packs
/// the entries of one key into a single quoted value split by `;`, and both
/// come out of here the same way. Section headers such as TOML's `[General]`
/// carry no `=` and fall away.
fn entries(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .map(|line| line.split_once('#').map_or(line, |(code, _)| code))
        .filter_map(|line| line.split_once('='))
        .flat_map(|(key, value)| {
            let key = key.trim();
            unquote(value.trim())
                .split(';')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(move |value| (key, value))
        })
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix(['\'', '"'])
        .and_then(|inner| inner.strip_suffix(['\'', '"']))
        .unwrap_or(value)
}

pub struct Manifest {
    pub name: Option<String>,
    pub cursors_directory: String,
}

impl Manifest {
    pub fn parse(text: &str) -> Option<Self> {
        let mut name = None;
        let mut cursors_directory = None;
        for (key, value) in entries(text) {
            match key {
                "name" => name = Some(value.to_owned()),
                "cursors_directory" => cursors_directory = Some(value.to_owned()),
                _ => {}
            }
        }
        Some(Self {
            name,
            cursors_directory: cursors_directory?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Resize {
    Bilinear,
    Nearest,
    #[default]
    None,
}

impl Resize {
    fn parse(value: &str) -> Self {
        match value {
            "bilinear" => Self::Bilinear,
            "nearest" => Self::Nearest,
            _ => Self::None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ImageEntry {
    /// The side length the image was authored at; zero for SVG.
    pub size: u32,
    pub file: String,
    pub delay: Duration,
}

impl ImageEntry {
    fn parse(value: &str) -> Option<Self> {
        let mut fields = value.split(',').map(str::trim);
        let size = fields.next()?.parse().ok()?;
        let file = fields.next().filter(|file| !file.is_empty())?.to_owned();
        let delay = fields
            .next()
            .and_then(|delay| delay.parse().ok())
            .map_or(Duration::ZERO, Duration::from_millis);
        Some(Self { size, file, delay })
    }

    pub fn is_vector(&self) -> bool {
        self.file.ends_with(".svg")
    }
}

#[derive(Debug)]
pub struct Meta {
    pub resize: Resize,
    /// Fractions of the image's side, from its top-left corner.
    pub hotspot: (f64, f64),
    /// How much of the requested size the drawn shape spans; the image is
    /// rasterised at `size / nominal_size`.
    pub nominal_size: f64,
    pub overrides: Vec<String>,
    pub images: Vec<ImageEntry>,
}

/// The frames to draw for one request, all at `side` pixels square.
pub struct Pick<'a> {
    pub side: u32,
    pub frames: Vec<&'a ImageEntry>,
}

impl Meta {
    pub fn parse(text: &str) -> Self {
        let mut meta = Self {
            resize: Resize::default(),
            hotspot: (0.0, 0.0),
            nominal_size: 1.0,
            overrides: Vec::new(),
            images: Vec::new(),
        };
        for (key, value) in entries(text) {
            match key {
                "resize_algorithm" => meta.resize = Resize::parse(value),
                "hotspot_x" => meta.hotspot.0 = value.parse().unwrap_or(0.0),
                "hotspot_y" => meta.hotspot.1 = value.parse().unwrap_or(0.0),
                "nominal_size" => {
                    meta.nominal_size = value
                        .parse()
                        .ok()
                        .filter(|size: &f64| *size > 0.0)
                        .unwrap_or(1.0)
                }
                "define_override" => meta.overrides.push(value.to_owned()),
                "define_size" => meta.images.extend(ImageEntry::parse(value)),
                _ => {}
            }
        }
        meta
    }

    /// Mirrors libhyprcursor: vector frames are drawn at exactly the side the
    /// request needs; bitmaps come from the exact size, else the smallest
    /// larger one, else the largest smaller one, and are resampled to fit. A
    /// shape that forbids resampling gets its nearest authored size as is.
    pub fn pick(&self, size: u32) -> Option<Pick<'_>> {
        let side = (f64::from(size) / self.nominal_size).round().max(1.0) as u32;

        if self.images.iter().any(ImageEntry::is_vector) {
            let frames = self
                .images
                .iter()
                .filter(|image| image.is_vector())
                .collect();
            return Some(Pick { side, frames });
        }

        let sizes = self.images.iter().map(|image| image.size);
        let authored = if self.resize == Resize::None {
            sizes.min_by_key(|authored| authored.abs_diff(side))?
        } else {
            let larger = sizes.clone().filter(|authored| *authored >= side).min();
            larger.or_else(|| sizes.max())?
        };
        let frames = self
            .images
            .iter()
            .filter(|image| image.size == authored)
            .collect();
        let side = if self.resize == Resize::None {
            authored
        } else {
            side
        };
        Some(Pick { side, frames })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HL: &str = "resize_algorithm = bilinear\n\n# a comment\nhotspot_x = 0.31\nhotspot_y = 0.12\ndefine_override = default\ndefine_override = arrow\ndefine_size = 0, left_ptr.svg\n";

    const TOML: &str = "[General]\nresize_algorithm = 'nearest'\nhotspot_x = 0.5\nhotspot_y = 0.25\nnominal_size = 0.5\ndefine_override = 'wait;watch'\ndefine_size = '24,wait-1.png,200; 24,wait-2.png,200;48,big.png'\n";

    #[test]
    fn hyprlang_meta_parses() {
        let meta = Meta::parse(HL);
        assert_eq!(meta.resize, Resize::Bilinear);
        assert_eq!(meta.hotspot, (0.31, 0.12));
        assert_eq!(meta.overrides, ["default", "arrow"]);
        assert_eq!(
            meta.images,
            [ImageEntry {
                size: 0,
                file: "left_ptr.svg".into(),
                delay: Duration::ZERO
            }]
        );
    }

    #[test]
    fn toml_meta_packs_entries_with_semicolons() {
        let meta = Meta::parse(TOML);
        assert_eq!(meta.resize, Resize::Nearest);
        assert_eq!(meta.nominal_size, 0.5);
        assert_eq!(meta.overrides, ["wait", "watch"]);
        assert_eq!(meta.images.len(), 3);
        assert_eq!(meta.images[1].file, "wait-2.png");
        assert_eq!(meta.images[1].delay, Duration::from_millis(200));
    }

    #[test]
    fn manifests_in_both_spellings_parse() {
        let hl = Manifest::parse("name = macOS\ncursors_directory = hyprcursors\n").unwrap();
        assert_eq!(hl.name.as_deref(), Some("macOS"));
        assert_eq!(hl.cursors_directory, "hyprcursors");
        let toml = Manifest::parse("[General]\nname = 'x'\ncursors_directory = \"c\"\n").unwrap();
        assert_eq!(toml.cursors_directory, "c");
        assert!(Manifest::parse("name = nothing else").is_none());
    }

    #[test]
    fn vector_frames_follow_the_nominal_size() {
        let meta = Meta::parse(HL);
        let pick = meta.pick(30).unwrap();
        assert_eq!(pick.side, 30);
        assert_eq!(pick.frames.len(), 1);

        let meta = Meta::parse("nominal_size = 0.5\ndefine_size = 0, a.svg");
        assert_eq!(meta.pick(24).unwrap().side, 48);
    }

    #[test]
    fn bitmaps_prefer_the_smallest_larger_size() {
        let meta = Meta::parse(
            "resize_algorithm = bilinear\ndefine_size = 24, a.png\ndefine_size = 32, b.png\ndefine_size = 64, c.png",
        );
        let pick = meta.pick(30).unwrap();
        assert_eq!((pick.side, pick.frames[0].file.as_str()), (30, "b.png"));
        assert_eq!(meta.pick(80).unwrap().frames[0].file, "c.png");
        assert_eq!(meta.pick(24).unwrap().frames[0].file, "a.png");
    }

    #[test]
    fn unresampled_bitmaps_keep_their_authored_size() {
        let meta = Meta::parse("define_size = 24, a.png\ndefine_size = 48, b.png");
        let pick = meta.pick(30).unwrap();
        assert_eq!((pick.side, pick.frames[0].file.as_str()), (24, "a.png"));
    }

    #[test]
    fn animations_keep_every_frame_of_one_size() {
        let meta = Meta::parse(TOML);
        let pick = meta.pick(12).unwrap();
        assert_eq!(pick.frames.len(), 2);
    }
}

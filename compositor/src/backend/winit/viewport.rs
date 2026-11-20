//! One nested output: a slice of the host window with its own `Output`.
//!
//! `CROWN_NESTED_OUTPUTS=N` (1 to 4) splits the window into N side-by-side
//! viewports, named `winit-1` to `winit-N`, or `winit` when there is one.
//! `CROWN_NESTED_NAMES=eDP-1,HDMI-A-1` gives them connector names instead.

use std::time::Duration;

use smithay::{
    backend::renderer::{
        damage::OutputDamageTracker, element::texture::TextureRenderBuffer, gles::GlesTexture,
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::{
    rendering::{blur::BlurCache, corners::CornerMemory, warp::WarpCanvases},
    shell::{
        monitor::{OutputDescriptor, Presence},
        output_kind::OutputKind,
    },
    state::DisabledHead,
};

pub const REFRESH_RATE: i32 = 60_000;
const MAX_VIEWPORTS: usize = 4;
const COUNT_VARIABLE: &str = "CROWN_NESTED_OUTPUTS";
const NAMES_VARIABLE: &str = "CROWN_NESTED_NAMES";
const SINGLE_NAME: &str = "winit";
const MAKE: &str = "Crownpositor";
const MODEL: &str = "Winit";

/// Whether the shell has the viewport's output, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Connected,
    /// Switched off through output management; listed as a disabled head.
    Disabled,
    /// Unplugged: not listed at all.
    Disconnected,
}

pub struct Viewport {
    pub name: String,
    /// The last output the shell had for this viewport. Stale unless connected.
    pub output: Output,
    pub damage_tracker: OutputDamageTracker,
    pub blur: BlurCache,
    pub corners: CornerMemory,
    pub canvases: WarpCanvases,
    /// The viewport's last frame, kept between frames. Only a split window
    /// has one; it is created on the first render after a resize.
    pub target: Option<TextureRenderBuffer<GlesTexture>>,
    pub rect: Rectangle<i32, Physical>,
    pub dirty: bool,
    pub connection: Connection,
    /// Where the output was when it was last switched off or unplugged.
    pub last_position: Option<Point<i32, Logical>>,
}

impl Viewport {
    pub fn new(
        name: String,
        output: Output,
        rect: Rectangle<i32, Physical>,
        connection: Connection,
    ) -> Self {
        Self {
            name,
            damage_tracker: OutputDamageTracker::from_output(&output),
            output,
            blur: BlurCache::default(),
            corners: CornerMemory::default(),
            canvases: WarpCanvases::default(),
            target: None,
            rect,
            dirty: true,
            connection,
            last_position: None,
        }
    }

    /// Starts over with the output the shell made for the same descriptor.
    pub fn reconnect(&mut self, output: Output) {
        let last_position = self.last_position;
        *self = Self::new(
            std::mem::take(&mut self.name),
            output,
            self.rect,
            Connection::Connected,
        );
        self.last_position = last_position;
    }

    pub fn is_connected(&self) -> bool {
        self.connection == Connection::Connected
    }

    pub fn mode(&self) -> Mode {
        mode_of(self.rect.size)
    }

    pub fn descriptor(&self, transform: Transform) -> OutputDescriptor {
        descriptor(&self.name, self.mode(), transform)
    }

    pub fn disabled_head(&self) -> DisabledHead {
        DisabledHead {
            name: self.name.clone(),
            identity: self.name.clone(),
            make: Some(MAKE.to_owned()),
            model: Some(MODEL.to_owned()),
            serial: None,
            physical_mm: (0, 0),
            modes: vec![self.mode()],
            preferred: Some(0),
            vrr_capable: false,
            kind: OutputKind::Nested,
            position: self.last_position,
        }
    }

    /// The layout point under `window_point`, for a monitor at `origin`.
    pub fn layout_point(
        &self,
        window_point: Point<f64, Physical>,
        origin: Point<i32, Logical>,
    ) -> Option<Point<f64, Logical>> {
        let scale = self.output.current_scale().fractional_scale();
        window_to_layout(window_point, self.rect, origin, scale)
    }
}

pub fn mode_of(size: Size<i32, Physical>) -> Mode {
    Mode {
        size,
        refresh: REFRESH_RATE,
    }
}

pub fn descriptor(name: &str, mode: Mode, transform: Transform) -> OutputDescriptor {
    OutputDescriptor {
        name: name.to_owned(),
        physical: PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: MAKE.into(),
            model: MODEL.into(),
            serial_number: "Unknown".into(),
        },
        modes: vec![mode],
        preferred: Some(mode),
        current: mode,
        native_transform: transform,
        refresh_interval: Some(Duration::from_nanos(
            1_000_000_000_000 / REFRESH_RATE as u64,
        )),
        serial: None,
        edid: None,
        presence: Presence::Shown,
        kind: OutputKind::Nested,
    }
}

/// The viewport names the environment asks for.
pub fn viewport_names() -> Vec<String> {
    let count = parse_count(std::env::var(COUNT_VARIABLE).ok().as_deref());
    parse_names(std::env::var(NAMES_VARIABLE).ok().as_deref(), count)
        .unwrap_or_else(|| default_names(count))
}

fn parse_count(value: Option<&str>) -> usize {
    let Some(value) = value else {
        return 1;
    };
    match value.trim().parse() {
        Ok(count) if (1..=MAX_VIEWPORTS).contains(&count) => count,
        _ => {
            tracing::warn!(
                value,
                "{COUNT_VARIABLE} must be 1 to {MAX_VIEWPORTS}, using 1"
            );
            1
        }
    }
}

fn parse_names(value: Option<&str>, count: usize) -> Option<Vec<String>> {
    let names: Vec<String> = value?
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    if names.len() == count && unique.len() == count {
        return Some(names);
    }
    tracing::warn!(
        value,
        count,
        "{NAMES_VARIABLE} must name each nested output once, ignoring it"
    );
    None
}

fn default_names(count: usize) -> Vec<String> {
    match count {
        1 => vec![SINGLE_NAME.to_owned()],
        _ => (1..=count)
            .map(|number| format!("{SINGLE_NAME}-{number}"))
            .collect(),
    }
}

/// Side-by-side slices of the window, as even as whole pixels allow.
pub fn split_window(size: Size<i32, Physical>, count: usize) -> Vec<Rectangle<i32, Physical>> {
    let Ok(parts) = i32::try_from(count.max(1)) else {
        return vec![Rectangle::from_size(size)];
    };
    (0..parts)
        .map(|index| {
            let left = size.w * index / parts;
            let right = size.w * (index + 1) / parts;
            Rectangle::new((left, 0).into(), (right - left, size.h).into())
        })
        .collect()
}

fn window_to_layout(
    window_point: Point<f64, Physical>,
    rect: Rectangle<i32, Physical>,
    origin: Point<i32, Logical>,
    scale: f64,
) -> Option<Point<f64, Logical>> {
    let rect = rect.to_f64();
    if !rect.contains(window_point) {
        return None;
    }
    Some(origin.to_f64() + (window_point - rect.loc).to_logical(scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_bad_count_means_one_viewport() {
        assert_eq!(parse_count(None), 1);
        for bad in ["0", "5", "two", ""] {
            assert_eq!(parse_count(Some(bad)), 1, "{bad:?}");
        }
        assert_eq!(parse_count(Some("3")), 3);
    }

    #[test]
    fn one_viewport_keeps_the_old_name() {
        assert_eq!(default_names(1), ["winit"]);
        assert_eq!(default_names(2), ["winit-1", "winit-2"]);
    }

    #[test]
    fn names_must_match_the_count_and_differ() {
        assert_eq!(
            parse_names(Some("eDP-1, HDMI-A-1"), 2),
            Some(vec!["eDP-1".to_owned(), "HDMI-A-1".to_owned()])
        );
        assert_eq!(parse_names(Some("eDP-1"), 2), None);
        assert_eq!(parse_names(Some("DP-1,DP-1"), 2), None);
        assert_eq!(parse_names(None, 2), None);
    }

    #[test]
    fn the_split_covers_the_window_without_gaps() {
        let rects = split_window((1001, 600).into(), 3);
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0].loc, (0, 0).into());
        for pair in rects.windows(2) {
            assert_eq!(pair[0].loc.x + pair[0].size.w, pair[1].loc.x);
        }
        assert_eq!(rects[2].loc.x + rects[2].size.w, 1001);
        assert!(rects.iter().all(|rect| rect.size.h == 600));
    }

    #[test]
    fn a_window_point_maps_through_its_viewport() {
        let rect = Rectangle::new((500, 0).into(), (500, 400).into());
        let origin = Point::from((250, 0));
        assert_eq!(
            window_to_layout((700.0, 100.0).into(), rect, origin, 2.0),
            Some((350.0, 50.0).into())
        );
        assert_eq!(
            window_to_layout((499.0, 100.0).into(), rect, origin, 2.0),
            None
        );
    }
}

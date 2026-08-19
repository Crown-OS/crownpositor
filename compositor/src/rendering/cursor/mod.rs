//! The pointer cursor.
//!
//! A Wayland compositor owns its cursor outright — nothing else on the system
//! draws it — so this module is the whole of it. Two things can be on screen:
//!
//! * a *named* shape ([`CursorIcon`]), resolved through the theme chain in
//!   [`source`] and rasterised once per icon and buffer scale into a
//!   [`MemoryRenderBuffer`];
//! * a client's own *surface*, handed over through `wl_pointer.set_cursor` and
//!   drawn like any other surface tree, offset by the hotspot it declared.
//!
//! Both are tagged [`Kind::Cursor`], which is what lets `DrmCompositor` promote
//! them onto the hardware cursor plane — a mouse move then costs one atomic
//! commit instead of recompositing the screen.
//!
//! The theme format is behind [`CursorSource`]: this module does the caching,
//! the scale arithmetic and the render element, and knows nothing about file
//! formats. Adding hyprcursor means adding one implementation of that trait and
//! one line in [`Cursor::sources`].

pub mod builtin;
pub mod source;
pub mod xcursor;

use std::collections::HashMap;

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
        },
    },
    input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Logical, Physical, Point, Scale, Transform},
    wayland::compositor::with_states,
};

use crate::rendering::cursor::{
    builtin::BuiltinSource,
    source::{CursorSource, RawImage},
    xcursor::XCursorSource,
};

/// Matches the X11/GTK default, and the size every theme ships.
const DEFAULT_SIZE: u32 = 24;

/// The session's cursor: what to draw, and the images to draw it from.
pub struct Cursor {
    /// Consulted in order, first hit wins. The last entry always answers, so
    /// there is no "no cursor" outcome.
    sources: Vec<Box<dyn CursorSource>>,
    /// Nominal *logical* size; the buffer scale multiplies it.
    size: u32,
    /// Keyed on icon and buffer scale. Rasterising is file I/O plus a parse, so
    /// it must not happen per frame — and a `None` records a shape no source
    /// could produce, which must not be retried per frame either.
    cache: HashMap<(CursorIcon, i32), Option<Image>>,
    /// What the focused client last asked for.
    pub status: CursorImageStatus,
}

/// A rasterised cursor image, ready to draw.
struct Image {
    buffer: MemoryRenderBuffer,
    /// Where the hot point sits inside the image, in logical pixels from its
    /// top-left corner.
    hotspot: Point<f64, Logical>,
}

impl Cursor {
    pub fn new() -> Self {
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|size| size.parse::<u32>().ok())
            .filter(|size| *size > 0)
            .unwrap_or(DEFAULT_SIZE);

        let sources = Self::sources();
        tracing::info!(
            size,
            chain = ?sources.iter().map(|source| source.describe()).collect::<Vec<_>>(),
            "cursor theme chain"
        );

        Self {
            sources,
            size,
            cache: HashMap::new(),
            status: CursorImageStatus::default_named(),
        }
    }

    /// The theme chain, most preferred first.
    ///
    /// The order is the whole configuration: a new format goes in ahead of the
    /// ones it should win over, and shapes it does not carry fall through to
    /// them. [`BuiltinSource`] must stay last — it answers everything, so
    /// nothing after it would ever be reached.
    fn sources() -> Vec<Box<dyn CursorSource>> {
        vec![Box::new(XCursorSource::from_env()), Box::new(BuiltinSource)]
    }

    /// Drops a surface cursor whose client is gone. Without this the pointer
    /// keeps a dead surface as its image and disappears for good.
    pub fn refresh(&mut self) {
        if let CursorImageStatus::Surface(surface) = &self.status
            && !surface.alive()
        {
            self.status = CursorImageStatus::default_named();
        }
    }

    /// The buffer scale to rasterise for. Integer, because themes ship discrete
    /// sizes; rounding up keeps a fractional-scale output sharp at the cost of a
    /// marginally smaller cursor.
    pub fn buffer_scale(scale: Scale<f64>) -> i32 {
        (scale.x.max(scale.y).ceil() as i32).max(1)
    }

    /// The image for `icon` at `scale`, walking the source chain on a miss.
    fn image(&mut self, icon: CursorIcon, scale: i32) -> Option<&Image> {
        // Split so the cache can be borrowed mutably while the chain is read.
        let Self {
            sources,
            size,
            cache,
            ..
        } = self;
        let size = *size * scale as u32;

        cache
            .entry((icon, scale))
            .or_insert_with(|| {
                let raw = sources.iter().find_map(|source| {
                    let image = source.shape(icon, size)?;
                    tracing::debug!(
                        cursor = icon.name(),
                        source = %source.describe(),
                        "resolved cursor shape"
                    );
                    Some(image)
                });
                // Only reachable if even the built-in arrow failed to
                // rasterise, which is an allocation failure, not a missing
                // theme.
                if raw.is_none() {
                    tracing::warn!(cursor = icon.name(), "no source could draw this cursor");
                }
                raw.map(|raw| Image::new(&raw, scale))
            })
            .as_ref()
    }

    /// Pushes the cursor's elements for one output, front of everything.
    ///
    /// `pointer` is global logical; `monitor` is the output being drawn and its
    /// origin is what makes the result output-local. Nothing is pushed when the
    /// pointer is on another output or a client asked for no cursor at all.
    pub fn render<R, E>(
        &mut self,
        elements: &mut Vec<E>,
        renderer: &mut R,
        output_location: Point<i32, Logical>,
        pointer: Point<f64, Logical>,
        scale: Scale<f64>,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        E: From<WaylandSurfaceRenderElement<R>> + From<MemoryRenderBufferRenderElement<R>>,
    {
        self.refresh();
        let local = pointer - output_location.to_f64();

        // Cloned so the `Surface` arm's borrow does not outlive the `Named`
        // arm's need for `&mut self`. A `WlSurface` clone is a refcount bump.
        match self.status.clone() {
            CursorImageStatus::Hidden => {}
            CursorImageStatus::Surface(surface) => {
                let hotspot = surface_hotspot(&surface);
                let location: Point<i32, Physical> =
                    (local - hotspot.to_f64()).to_physical_precise_round(scale);
                let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
                    render_elements_from_surface_tree(
                        renderer,
                        &surface,
                        location,
                        scale,
                        1.0,
                        Kind::Cursor,
                    );
                elements.extend(surfaces.into_iter().map(E::from));
            }
            CursorImageStatus::Named(icon) => {
                let Some(image) = self.image(icon, Self::buffer_scale(scale)) else {
                    return;
                };
                // Physical and fractional: rounding the hotspot to a logical
                // pixel would make the cursor visibly stutter on a scaled
                // output.
                let location = (local - image.hotspot).to_physical(scale);
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    location,
                    &image.buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    Ok(element) => elements.push(E::from(element)),
                    // An import failure is per-frame, not permanent, so the
                    // cache entry stays: the next frame tries again.
                    Err(_) => tracing::warn!(cursor = icon.name(), "failed to import the cursor"),
                }
            }
        }
    }

    /// Lets an animated client cursor draw its next frame. A cursor surface is
    /// not in the shell model, so the backends' window and layer passes never
    /// reach it.
    pub fn send_frame(
        &self,
        output: &Output,
        time: std::time::Duration,
        throttle: Option<std::time::Duration>,
    ) {
        if let CursorImageStatus::Surface(surface) = &self.status {
            smithay::desktop::utils::send_frames_surface_tree(
                surface,
                output,
                time,
                throttle,
                |_, _| Some(output.clone()),
            );
        }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}

impl Image {
    fn new(raw: &RawImage, scale: i32) -> Self {
        Self {
            buffer: MemoryRenderBuffer::from_slice(
                &raw.pixels,
                raw.format,
                (raw.width, raw.height),
                scale,
                Transform::Normal,
                None,
            ),
            // The source works in its own pixels; the buffer scale is what
            // turns those into the logical size the element is drawn at.
            hotspot: Point::from((raw.hotspot.0 / scale as f64, raw.hotspot.1 / scale as f64)),
        }
    }
}

/// Where a client's cursor surface wants the hot point. Absent role data means
/// the surface was never set as a cursor, in which case its own origin is the
/// only sensible answer.
fn surface_hotspot(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CursorImageSurfaceData>()
            .and_then(|data| data.lock().ok().map(|attributes| attributes.hotspot))
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_scale_rounds_up_and_never_reaches_zero() {
        assert_eq!(Cursor::buffer_scale(Scale::from(1.0)), 1);
        assert_eq!(Cursor::buffer_scale(Scale::from(1.25)), 2);
        assert_eq!(Cursor::buffer_scale(Scale::from(2.0)), 2);
        // A misconfigured output must not divide the hotspot by zero.
        assert_eq!(Cursor::buffer_scale(Scale::from(0.0)), 1);
    }

    #[test]
    fn the_chain_ends_in_a_source_that_always_answers() {
        // The invariant the whole "cursor is never missing" guarantee rests on.
        let sources = Cursor::sources();
        let last = sources.last().expect("the chain must not be empty");
        assert!(last.shape(CursorIcon::Default, DEFAULT_SIZE).is_some());
    }

    #[test]
    fn a_named_shape_always_resolves_to_an_image() {
        // Runs on machines with and without a cursor theme installed, which is
        // exactly the point: the fallback covers the difference.
        let mut cursor = Cursor::new();
        for scale in 1..=2 {
            assert!(cursor.image(CursorIcon::Default, scale).is_some());
            assert!(cursor.image(CursorIcon::Text, scale).is_some());
        }
    }

    #[test]
    fn resolving_a_shape_twice_hits_the_cache() {
        let mut cursor = Cursor::new();
        cursor.image(CursorIcon::Default, 1);
        cursor.image(CursorIcon::Default, 1);
        assert_eq!(cursor.cache.len(), 1);
    }

    #[test]
    fn the_hotspot_is_scaled_into_logical_pixels() {
        // A HiDPI image's hot point is in its own dense pixels; drawn at the
        // matching scale it has to land back on the same logical spot, or the
        // click point drifts from the drawn tip.
        let raw = RawImage {
            pixels: vec![0; 4 * 48 * 48],
            format: smithay::backend::allocator::Fourcc::Argb8888,
            width: 48,
            height: 48,
            hotspot: (8.0, 12.0),
        };
        let image = Image::new(&raw, 2);
        assert_eq!(image.hotspot, Point::from((4.0, 6.0)));
    }
}

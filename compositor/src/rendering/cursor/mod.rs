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
//! the scale arithmetic, animation and the render element, and knows nothing
//! about file formats. Hyprcursor is tried first, XCursor after it, and the
//! built-in arrow last.

pub mod builtin;
pub mod hyprcursor;
pub mod source;
pub mod xcursor;

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use calloop::ping::Ping;

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
        },
    },
    input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Logical, Physical, Point, Scale, Transform},
    wayland::compositor::with_states,
};

use crate::rendering::{
    cursor::{
        builtin::BuiltinSource,
        hyprcursor::HyprcursorSource,
        source::{CursorSource, RawFrame, RawImage, RawShape},
        xcursor::XCursorSource,
    },
    surface_tree::{SurfaceElement, surface_elements},
    unstretched::Unstretched,
};

/// Matches the X11/GTK default, and the size every theme ships.
const DEFAULT_SIZE: u32 = 24;

/// The session's cursor: what to draw, and the images to draw it from.
pub struct Cursor {
    /// Consulted in order, first hit wins. The last entry always answers, so
    /// there is no "no cursor" outcome.
    sources: Vec<Box<dyn CursorSource>>,
    /// The theme `appearance.cursor_theme` names; empty defers to the
    /// environment.
    theme: String,
    /// Nominal *logical* size; an output's scale multiplies it into pixels.
    size: u32,
    /// Keyed on icon and pixel size. Rasterising is file I/O plus a parse, so
    /// it must not happen per frame — and a `None` records a shape no source
    /// could produce, which must not be retried per frame either.
    cache: HashMap<(CursorIcon, u32), Option<Shape>>,
    /// What the focused client last asked for.
    pub status: CursorImageStatus,
    /// Nothing is drawn while set: a locked pointer points at nothing.
    pub suppressed: bool,
    animation: Animation,
}

/// A rasterised shape, ready to draw one image pixel per output pixel.
struct Shape {
    frames: Vec<Frame>,
    /// One full cycle; zero for a still shape.
    period: Duration,
}

struct Frame {
    buffer: MemoryRenderBuffer,
    /// Where the hot point sits inside the image, in its pixels from its
    /// top-left corner.
    hotspot: Point<f64, Physical>,
    delay: Duration,
}

/// The clock of an animated shape, and the wake-up that redraws it.
///
/// Animations restart whenever the shape changes, so a spinner always begins
/// on its first frame. The redraw is requested through `wake` once per frame
/// change and only while an animated shape is actually drawn, so a still
/// cursor costs no timers at all.
struct Animation {
    icon: Option<CursorIcon>,
    since: Instant,
    wake: Option<Ping>,
    due: Option<Instant>,
}

impl Cursor {
    pub fn new(theme: &str) -> Self {
        let size = ["XCURSOR_SIZE", "HYPRCURSOR_SIZE"]
            .iter()
            .filter_map(|name| std::env::var(name).ok()?.parse::<u32>().ok())
            .find(|size| *size > 0)
            .unwrap_or(DEFAULT_SIZE);

        let sources = Self::sources(theme);
        tracing::info!(
            size,
            chain = ?sources.iter().map(|source| source.describe()).collect::<Vec<_>>(),
            "cursor theme chain"
        );

        Self {
            sources,
            theme: theme.to_owned(),
            size,
            cache: HashMap::new(),
            status: CursorImageStatus::default_named(),
            suppressed: false,
            animation: Animation::idle(),
        }
    }

    /// The theme chain, most preferred first.
    ///
    /// A hyprcursor theme by the configured name goes first, and XCursor
    /// answers whatever it does not carry: from the same name when hyprcursor
    /// had none, otherwise from the environment's theme. [`BuiltinSource`]
    /// must stay last — it answers everything, so nothing after it would ever
    /// be reached.
    fn sources(theme: &str) -> Vec<Box<dyn CursorSource>> {
        let configured = Some(theme).filter(|theme| !theme.is_empty());
        let hyprcursor = HyprcursorSource::new(configured);
        let xcursor_theme = configured.filter(|_| hyprcursor.is_none());

        let mut sources: Vec<Box<dyn CursorSource>> = Vec::with_capacity(3);
        if let Some(hyprcursor) = hyprcursor {
            sources.push(Box::new(hyprcursor));
        }
        sources.push(Box::new(XCursorSource::new(xcursor_theme)));
        sources.push(Box::new(BuiltinSource));
        sources
    }

    /// Switches to the theme `appearance.cursor_theme` now names. Returns
    /// whether anything changed, i.e. whether the cursor needs a redraw.
    pub fn set_theme(&mut self, theme: &str) -> bool {
        if self.theme == theme {
            return false;
        }
        let mut fresh = Self::new(theme);
        fresh.status = std::mem::replace(&mut self.status, CursorImageStatus::Hidden);
        fresh.suppressed = self.suppressed;
        fresh.animation.wake = self.animation.wake.take();
        *self = fresh;
        true
    }

    /// Lets animated shapes ask for redraws: `wake` is pinged whenever the
    /// drawn frame is about to change, and the owner of the event loop then
    /// waits until [`Cursor::animation_due`] and redraws the pointer.
    pub fn set_animation_wake(&mut self, wake: Ping) {
        self.animation.wake = Some(wake);
    }

    /// When the next frame of the drawn animation is due.
    pub fn animation_due(&self) -> Option<Instant> {
        self.animation.due
    }

    /// The due frame is being drawn, so the redraw that shows it may ask for
    /// the one after.
    pub fn animation_frame_reached(&mut self) {
        self.animation.due = None;
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

    /// Takes the pointer image back from the clients, and says whether it had
    /// to.
    ///
    /// A client is free to hide the cursor, or to hand over a surface it later
    /// stops drawing into, and either leaves the compositor with nothing to
    /// draw. On the desktop that is the client's business — it is the thing
    /// under the pointer. Once the overview owns the pointer it is not, and
    /// there is no longer a client that will ever put the cursor back, so the
    /// compositor's own arrow wins: it is the one image that is always
    /// drawable.
    pub fn reclaim(&mut self) -> bool {
        if matches!(self.status, CursorImageStatus::Named(_)) {
            return false;
        }
        self.status = CursorImageStatus::default_named();
        true
    }

    /// How many pixels across the cursor is on an output of this scale.
    ///
    /// Exactly the nominal size times the scale, not times the scale rounded
    /// up: an image rasterised for 2x and squeezed into a 1.25x output is
    /// resampled, and a cursor is nearly all edges.
    fn pixel_size(&self, scale: Scale<f64>) -> u32 {
        (f64::from(self.size) * scale.x.max(scale.y))
            .round()
            .max(1.0) as u32
    }

    /// The shape for `icon` at `pixels` across, walking the source chain on a
    /// miss.
    fn shape(&mut self, icon: CursorIcon, pixels: u32) -> Option<&Shape> {
        // Split so the cache can be borrowed mutably while the chain is read.
        let Self { sources, cache, .. } = self;

        cache
            .entry((icon, pixels))
            .or_insert_with(|| {
                let raw = sources.iter().find_map(|source| {
                    let image = source.shape(icon, pixels)?;
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
                raw.map(|raw| Shape::new(&raw))
            })
            .as_ref()
    }

    /// The pixels of a named shape for an output of this scale, straight from
    /// the theme chain, first frame only. Uncached: only the capture path
    /// asks, and only when the shape changes.
    pub fn raw_image(&self, icon: CursorIcon, scale: Scale<f64>) -> Option<RawImage> {
        let pixels = self.pixel_size(scale);
        self.sources
            .iter()
            .find_map(|source| source.shape(icon, pixels))
            .map(RawShape::into_first)
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
        E: From<SurfaceElement<R>> + From<Unstretched<MemoryRenderBufferRenderElement<R>>>,
    {
        self.refresh();
        if self.suppressed {
            return;
        }
        let local = pointer - output_location.to_f64();

        // Cloned so the `Surface` arm's borrow does not outlive the `Named`
        // arm's need for `&mut self`. A `WlSurface` clone is a refcount bump.
        match self.status.clone() {
            CursorImageStatus::Hidden => {}
            CursorImageStatus::Surface(surface) => {
                let hotspot = surface_hotspot(&surface);
                let location: Point<i32, Physical> =
                    (local - hotspot.to_f64()).to_physical_precise_round(scale);
                let surfaces =
                    surface_elements(renderer, &surface, location, scale, 1.0, Kind::Cursor);
                elements.extend(surfaces.into_iter().map(E::from));
            }
            CursorImageStatus::Named(icon) => {
                let now = Instant::now();
                let elapsed = self.animation.restart_on_change(icon, now);
                let pixels = self.pixel_size(scale);
                let Some(shape) = self.shape(icon, pixels) else {
                    return;
                };
                let (frame, next) = shape.frame_at(elapsed);
                // On a whole physical pixel, because it is drawn 1:1; rounding
                // to a logical pixel instead would make it visibly stutter.
                let location = (local.to_physical(scale) - frame.hotspot).to_i32_round::<i32>();
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    location.to_f64(),
                    &frame.buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    Ok(element) => elements.push(E::from(Unstretched::at_own_size(element, scale))),
                    // An import failure is per-frame, not permanent, so the
                    // cache entry stays: the next frame tries again.
                    Err(_) => tracing::warn!(cursor = icon.name(), "failed to import the cursor"),
                }
                if let Some(next) = next {
                    self.animation.request(now + next);
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
        Self::new("")
    }
}

impl Animation {
    fn idle() -> Self {
        Self {
            icon: None,
            since: Instant::now(),
            wake: None,
            due: None,
        }
    }

    /// How far into its animation `icon` is, starting over when it differs
    /// from the shape drawn last.
    fn restart_on_change(&mut self, icon: CursorIcon, now: Instant) -> Duration {
        if self.icon != Some(icon) {
            self.icon = Some(icon);
            self.since = now;
        }
        now - self.since
    }

    /// One wake-up per frame change, however many outputs draw the cursor.
    fn request(&mut self, due: Instant) {
        let Some(wake) = &self.wake else {
            return;
        };
        if self.due.is_none() {
            self.due = Some(due);
            wake.ping();
        }
    }
}

impl Shape {
    fn new(raw: &RawShape) -> Self {
        let frames: Vec<Frame> = raw.frames().iter().map(Frame::new).collect();
        let period = if frames.len() > 1 {
            frames.iter().map(|frame| frame.delay).sum()
        } else {
            Duration::ZERO
        };
        Self { frames, period }
    }

    /// The frame showing `elapsed` into the animation, and how long until the
    /// next one replaces it; `None` for a still shape.
    fn frame_at(&self, elapsed: Duration) -> (&Frame, Option<Duration>) {
        let first = &self.frames[0];
        if self.period.is_zero() {
            return (first, None);
        }
        let mut into_cycle =
            Duration::from_nanos((elapsed.as_nanos() % self.period.as_nanos()) as u64);
        for frame in &self.frames {
            if into_cycle < frame.delay {
                return (frame, Some(frame.delay - into_cycle));
            }
            into_cycle -= frame.delay;
        }
        (first, Some(first.delay))
    }
}

impl Frame {
    fn new(raw: &RawFrame) -> Self {
        let image = &raw.image;
        Self {
            buffer: MemoryRenderBuffer::from_slice(
                &image.pixels,
                image.format,
                (image.width, image.height),
                1,
                Transform::Normal,
                None,
            ),
            hotspot: Point::from(image.hotspot),
            delay: raw.delay,
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
    fn the_cursor_is_rasterised_for_the_exact_scale() {
        let cursor = Cursor::default();
        let nominal = f64::from(cursor.size);
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let pixels = cursor.pixel_size(Scale::from(scale));
            assert_eq!(f64::from(pixels), (nominal * scale).round(), "at {scale}");
        }
        // A misconfigured output must still get a cursor.
        assert_eq!(cursor.pixel_size(Scale::from(0.0)), 1);
    }

    #[test]
    fn the_chain_ends_in_a_source_that_always_answers() {
        // The invariant the whole "cursor is never missing" guarantee rests on.
        let sources = Cursor::sources("");
        let last = sources.last().expect("the chain must not be empty");
        assert!(last.shape(CursorIcon::Default, DEFAULT_SIZE).is_some());
    }

    #[test]
    fn a_named_shape_always_resolves_to_an_image() {
        // Runs on machines with and without a cursor theme installed, which is
        // exactly the point: the fallback covers the difference.
        let mut cursor = Cursor::default();
        for pixels in [24, 30, 36, 48] {
            assert!(cursor.shape(CursorIcon::Default, pixels).is_some());
            assert!(cursor.shape(CursorIcon::Text, pixels).is_some());
        }
    }

    #[test]
    fn resolving_a_shape_twice_hits_the_cache() {
        let mut cursor = Cursor::default();
        cursor.shape(CursorIcon::Default, 24);
        cursor.shape(CursorIcon::Default, 24);
        assert_eq!(cursor.cache.len(), 1);
    }

    #[test]
    fn the_hotspot_stays_in_the_image_pixels_it_is_drawn_in() {
        // Drawn 1:1, the hot point must stay on the pixel the theme put it on,
        // or the click point drifts from the drawn tip.
        let raw = RawImage {
            pixels: vec![0; 4 * 30 * 30],
            format: smithay::backend::allocator::Fourcc::Argb8888,
            width: 30,
            height: 30,
            hotspot: (5.0, 7.5),
        };
        let frame = RawFrame {
            image: raw,
            delay: Duration::ZERO,
        };
        assert_eq!(Frame::new(&frame).hotspot, Point::from((5.0, 7.5)));
    }

    fn animation(delays_ms: &[u64]) -> Shape {
        let frames = delays_ms
            .iter()
            .map(|delay| RawFrame {
                image: RawImage {
                    pixels: vec![0; 4],
                    format: smithay::backend::allocator::Fourcc::Argb8888,
                    width: 1,
                    height: 1,
                    hotspot: (0.0, 0.0),
                },
                delay: Duration::from_millis(*delay),
            })
            .collect();
        Shape::new(&RawShape::new(frames).expect("valid frames"))
    }

    #[test]
    fn an_animation_cycles_through_its_frames() {
        let shape = animation(&[20, 30]);
        let at = |ms| {
            let (frame, next) = shape.frame_at(Duration::from_millis(ms));
            (frame.delay.as_millis(), next.map(|next| next.as_millis()))
        };
        assert_eq!(at(0), (20, Some(20)));
        assert_eq!(at(25), (30, Some(25)));
        assert_eq!(at(55), (20, Some(15)));
    }

    #[test]
    fn a_still_shape_never_asks_for_a_redraw() {
        let shape = animation(&[0]);
        assert!(shape.frame_at(Duration::from_secs(5)).1.is_none());
    }

    #[test]
    fn animation_wakes_once_per_frame_change() {
        let (ping, _source) = calloop::ping::make_ping().expect("ping");
        let mut cursor = Cursor::default();
        cursor.set_animation_wake(ping);
        let due = Instant::now();
        cursor.animation.request(due);
        cursor.animation.request(due + Duration::from_millis(5));
        assert_eq!(cursor.animation_due(), Some(due));
        cursor.animation_frame_reached();
        assert_eq!(cursor.animation_due(), None);
    }
}

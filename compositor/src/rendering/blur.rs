//! GPU blur for `ext-background-effect-v1` surfaces.
//!
//! The model is Hyprland's cached pre-blur (its `new_optimizations` path):
//! once per output, everything that lives *under* the window stack — the
//! wallpaper and the Background/Bottom layer-shell surfaces — is rendered into
//! an offscreen texture and pushed through a dual-kawase pyramid. A surface
//! that committed a blur region then gets a [`BlurBackdrop`] element per
//! rectangle of that region, drawn behind its own contents and sampling the
//! blurred texture where it sits on screen, all masked by the same rounded
//! rectangle the surface itself is clipped with.
//!
//! Why this shape:
//!
//! * **Cost is paid on wallpaper damage, not per frame.** The offscreen scene
//!   has its own [`OutputDamageTracker`]; when nothing under the windows
//!   changed, `update` is a damage query and nothing else. A static wallpaper
//!   means the kawase chain runs approximately never.
//! * **Windows moving over the blur are free.** The backdrop samples a
//!   prerendered screen-space texture, so dragging a translucent window is
//!   the same recomposite any opaque window costs.
//! * **It degrades.** No shaders, no texture, an allocation failure — the
//!   backdrop element simply isn't emitted and windows draw as before.
//!
//! What it buys with that: a backdrop always shows the blurred *wallpaper*,
//! never the windows in between. Behind a bar or a panel — which reserves its
//! space, so nothing else is ever under it — that is exactly right. A floating
//! window stacked over another one blurs the desktop rather than its
//! neighbour, which is the price of blurring once per output instead of once
//! per surface.
//!
//! Everything here talks to a plain [`GlesRenderer`]; on the KMS backend that
//! is the render node's GLES context under the multi-GPU wrapper (the same
//! one the rounded-corner shader binds through), so the blurred texture lives
//! exactly where the output's composition happens.

use std::sync::Mutex;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Frame as _, Offscreen, Renderer, Texture,
            damage::OutputDamageTracker,
            element::{
                Element, Id, Kind, RenderElement, UnderlyingStorage,
                surface::WaylandSurfaceRenderElement,
            },
            gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture},
            multigpu::{Error as MultiError, MultiFrame},
            utils::{CommitCounter, DamageSet, OpaqueRegions, with_renderer_surface_state},
        },
    },
    desktop::{Window, WindowSurface, layer_map_for_output},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::{compositor::with_states, shell::wlr_layer::Layer},
};

use config::Appearance;
use protocols::background_effect;

use crate::{
    backend::render::{GbmGlesApi, KmsRenderer},
    rendering::decorate::Backdrop,
    shaders::blur::BlurShaders,
    shell::{Shell, monitor::Monitor},
};

/// The layers whose surfaces may blur.
///
/// Bottom and Background are missing on purpose: they are the scene the blur
/// is computed *from*, so letting them blur would have them sample a texture
/// they are themselves inside — a feedback loop that smears more with every
/// frame. Panels and notifications, which is what actually wants glass, live
/// on Top and Overlay.
pub(crate) const BLURRABLE_LAYERS: [Layer; 2] = [Layer::Overlay, Layer::Top];

/// Runtime knobs for the blur pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurConfig {
    pub enabled: bool,
    /// Downsample depth of the kawase pyramid. Each pass halves the
    /// resolution, so perceived radius grows exponentially with this.
    pub passes: u8,
    /// Kawase tap spread, in (level-local) pixels. Fractional values are the
    /// point: the taps land between texels and the bilinear filter does the
    /// averaging.
    pub offset: f32,
    /// Dither strength applied when compositing, to hide gradient banding.
    pub noise: f32,
}

impl From<&Appearance> for BlurConfig {
    /// The file speaks in user units; the pipeline wants what the shader can
    /// hold, so this is where the narrowing and the sanity clamps happen.
    fn from(appearance: &Appearance) -> Self {
        Self {
            enabled: appearance.blur,
            passes: appearance.blur_passes.min(u8::MAX.into()) as u8,
            offset: appearance.blur_size.max(0.0) as f32,
            noise: appearance.blur_noise.clamp(0.0, 1.0) as f32,
        }
    }
}

impl Default for BlurConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            passes: 3,
            offset: 1.5,
            noise: 0.01,
        }
    }
}

impl BlurConfig {
    fn passes(&self) -> usize {
        // Below 1 the pyramid does not exist; above 8 the smallest level of
        // any real output is a pixel and the extra passes only burn time.
        self.passes.clamp(1, 8) as usize
    }

    /// Everything that invalidates an already-blurred texture when it changes.
    fn fingerprint(&self) -> (usize, u32) {
        (self.passes(), self.offset.to_bits())
    }
}

/// Errors from the blur pipeline. Callers log and drop the effect; none of
/// these may take a frame down.
#[derive(Debug, thiserror::Error)]
pub enum BlurError {
    #[error("the blur shaders are not compiled on this context")]
    MissingShaders,
    #[error("failed to allocate an offscreen texture: {0}")]
    Allocate(#[source] GlesError),
    #[error("offscreen render failed: {0}")]
    Render(String),
    #[error("kawase pass failed: {0}")]
    Kawase(#[source] GlesError),
}

/// The offscreen state for one output's blur: the scene capture, the kawase
/// pyramid, and the finished blurred texture windows sample from.
#[derive(Debug, Default)]
pub struct BlurBuffers {
    inner: Option<Buffers>,
    /// Bumped whenever `blurred` gets new content, so surfaces sampling it can
    /// tell "the same glass as last frame" from "re-blurred, repaint".
    serial: u64,
    fingerprint: Option<(usize, u32)>,
}

#[derive(Debug)]
struct Buffers {
    size: Size<i32, Physical>,
    scale: Scale<f64>,
    /// The un-blurred scene under the windows, kept alive so its damage
    /// tracker can re-render only what changed.
    scene: GlesTexture,
    tracker: OutputDamageTracker,
    /// Halving-resolution kawase levels, largest first.
    levels: Vec<GlesTexture>,
    /// Full-resolution result of the final upsample.
    blurred: GlesTexture,
    /// Whether `blurred` has ever been filled.
    primed: bool,
}

impl BlurBuffers {
    /// Re-renders the under-windows scene and, if anything about it changed,
    /// re-runs the kawase chain. Cheap when nothing changed.
    ///
    /// `elements` are the scene's contents in output-local physical
    /// coordinates, front-to-back, rendered against `clear_color` — the same
    /// conventions as the main pass.
    pub fn update(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
        scale: Scale<f64>,
        elements: &[WaylandSurfaceRenderElement<GlesRenderer>],
        clear_color: [f32; 4],
        config: &BlurConfig,
    ) -> Result<(), BlurError> {
        let shaders = BlurShaders::get(renderer).ok_or(BlurError::MissingShaders)?;
        let passes = config.passes();

        // (Re)build the texture set when the output or the pyramid shape
        // changed. Kept simple on purpose: mode switches are rare and a full
        // reallocation is microseconds next to the modeset around it.
        let rebuild = !self.inner.as_ref().is_some_and(|buffers| {
            buffers.size == size && buffers.scale == scale && buffers.levels.len() == passes
        });
        if rebuild {
            self.inner = Some(Buffers::allocate(renderer, size, scale, passes)?);
        }
        let Some(buffers) = self.inner.as_mut() else {
            // `allocate` either filled it or returned an error above.
            return Err(BlurError::Render("no offscreen buffers".into()));
        };

        // Render the scene with its own damage tracker. Age 1 after the first
        // fill: the same texture is reused, so only damage since the previous
        // update gets redrawn.
        let age = if buffers.primed { 1 } else { 0 };
        let scene_damaged = {
            let mut framebuffer = renderer
                .bind(&mut buffers.scene)
                .map_err(BlurError::Allocate)?;
            let result = buffers
                .tracker
                .render_output(renderer, &mut framebuffer, age, elements, clear_color)
                .map_err(|err| BlurError::Render(err.to_string()))?;
            result.damage.is_some_and(|damage| !damage.is_empty())
        };

        let config_changed = self.fingerprint != Some(config.fingerprint());
        tracing::trace!(
            scene_damaged,
            config_changed,
            primed = buffers.primed,
            sources = elements.len(),
            "blur pre-pass decision"
        );
        if buffers.primed && !scene_damaged && !config_changed {
            return Ok(());
        }

        // The kawase chain always runs over the whole texture: it only
        // executes when the scene actually changed, and the pyramid's small
        // levels make the full-surface passes cheap next to per-rect
        // bookkeeping across five coordinate spaces.
        let offset = config.offset.max(0.0);

        // Downsample: scene -> levels[0] -> ... -> levels[passes - 1].
        let mut source = buffers.scene.clone();
        for level in &mut buffers.levels {
            let destination_size = level.size();
            kawase_pass(
                renderer,
                &source,
                level,
                destination_size,
                &shaders,
                /* down */ true,
                offset,
            )?;
            source = level.clone();
        }

        // Upsample back: levels[n] -> levels[n - 1] -> ... -> levels[0].
        for index in (0..passes.saturating_sub(1)).rev() {
            let source = buffers.levels[index + 1].clone();
            let source_size = source.size();
            kawase_pass(
                renderer,
                &source,
                &mut buffers.levels[index],
                source_size,
                &shaders,
                /* down */ false,
                offset,
            )?;
        }

        // Final upsample to full resolution, where windows sample 1:1.
        let source = buffers.levels[0].clone();
        let source_size = source.size();
        kawase_pass(
            renderer,
            &source,
            &mut buffers.blurred,
            source_size,
            &shaders,
            /* down */ false,
            offset,
        )?;

        buffers.primed = true;
        self.fingerprint = Some(config.fingerprint());
        self.serial = self.serial.wrapping_add(1);
        // Debug rather than trace: this firing every frame is the blur
        // cache failing, and that is exactly what one greps for.
        tracing::debug!(?size, passes, "re-blurred the background scene");
        Ok(())
    }

    /// A handle for building backdrop elements this frame. `None` until the
    /// first successful [`update`](Self::update).
    pub fn source(&self, noise: f32) -> Option<BackdropSource> {
        let buffers = self.inner.as_ref().filter(|buffers| buffers.primed)?;
        Some(BackdropSource {
            texture: buffers.blurred.clone(),
            serial: self.serial,
            noise,
        })
    }

    /// Drops the offscreen textures, e.g. when blur got disabled.
    pub fn clear(&mut self) {
        self.inner = None;
        self.fingerprint = None;
    }
}

impl Buffers {
    fn allocate(
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
        scale: Scale<f64>,
        passes: usize,
    ) -> Result<Self, BlurError> {
        let buffer_size = Size::<i32, BufferCoords>::from((size.w.max(1), size.h.max(1)));
        let allocate = |renderer: &mut GlesRenderer, size: Size<i32, BufferCoords>| {
            Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, size)
                .map_err(BlurError::Allocate)
        };

        let scene = allocate(renderer, buffer_size)?;
        let blurred = allocate(renderer, buffer_size)?;
        let mut levels = Vec::with_capacity(passes);
        for pass in 0..passes {
            let shift = (pass + 1) as u32;
            let level_size = Size::from((
                (buffer_size.w >> shift).max(1),
                (buffer_size.h >> shift).max(1),
            ));
            levels.push(allocate(renderer, level_size)?);
        }

        Ok(Self {
            size,
            scale,
            scene,
            // Not `from_output`: the scene is captured untransformed, in the
            // same output-local space the element list uses, and the output's
            // transform is applied only by the *main* pass that samples it.
            tracker: OutputDamageTracker::new(size, scale, Transform::Normal),
            levels,
            blurred,
            primed: false,
        })
    }
}

/// One kawase pass: draw `source` over all of `destination` through the down-
/// or upsample program. `half_pixel` follows the dual-filter convention: half
/// a texel of the *smaller* texture involved (destination when downsampling,
/// source when upsampling).
fn kawase_pass(
    renderer: &mut GlesRenderer,
    source: &GlesTexture,
    destination: &mut GlesTexture,
    smaller_size: Size<i32, BufferCoords>,
    shaders: &BlurShaders,
    down: bool,
    offset: f32,
) -> Result<(), BlurError> {
    let half_pixel = (
        0.5 / smaller_size.w.max(1) as f32,
        0.5 / smaller_size.h.max(1) as f32,
    );
    let uniforms = BlurShaders::kawase_values(half_pixel, offset);
    let program = if down { &shaders.down } else { &shaders.up };

    let destination_size = destination.size();
    let target_size = Size::<i32, Physical>::from((destination_size.w, destination_size.h));
    let target_rect = Rectangle::from_size(target_size);
    let source_rect = Rectangle::from_size(source.size().to_f64());

    let mut framebuffer = renderer.bind(destination).map_err(BlurError::Kawase)?;
    let mut frame = renderer
        .render(&mut framebuffer, target_size, Transform::Normal)
        .map_err(BlurError::Kawase)?;
    frame
        .render_texture_from_to(
            source,
            source_rect,
            target_rect,
            &[target_rect],
            &[],
            Transform::Normal,
            1.0,
            Some(program),
            &uniforms,
        )
        .map_err(BlurError::Kawase)?;
    // The passes run in submission order on one context; the sync point of
    // the final frame is carried by the main pass that samples the result.
    let _sync = frame.finish().map_err(BlurError::Kawase)?;
    Ok(())
}

/// What a decorator needs to build backdrop elements for one output/frame.
#[derive(Debug, Clone)]
pub struct BackdropSource {
    texture: GlesTexture,
    serial: u64,
    noise: f32,
}

impl BackdropSource {
    /// Identifies this frame's blurred texture. Surfaces pair it with their
    /// region's generation to decide whether their backdrops changed.
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

/// One rectangle of the blurred glass behind a surface.
///
/// Samples [`BackdropSource`]'s screen-space texture at that rectangle, so it
/// holds physical coordinates directly — they were computed against the same
/// scale the texture was rendered at.
#[derive(Debug)]
pub struct BlurBackdrop {
    id: Id,
    commit: CommitCounter,
    texture: GlesTexture,
    /// `None` draws the sample unmasked (square corners, no dither) — the
    /// finish shader failing to compile must not drop the backdrop.
    program: Option<smithay::backend::renderer::gles::GlesTexProgram>,
    geometry: Rectangle<i32, Physical>,
    mask: Rectangle<i32, Physical>,
    radius: f32,
    noise: f32,
    alpha: f32,
}

impl BlurBackdrop {
    pub fn new(renderer: &GlesRenderer, source: &BackdropSource, params: Backdrop) -> Self {
        let program = BlurShaders::get(renderer).map(|shaders| shaders.finish);
        Self {
            id: params.id,
            commit: params.commit,
            texture: source.texture.clone(),
            program,
            geometry: params.geometry,
            mask: params.mask,
            radius: params.radius,
            noise: source.noise,
            alpha: params.alpha,
        }
    }

    /// The window rect in the blurred texture's buffer space (1:1 physical).
    fn source_rect(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::new(
            Point::from((self.geometry.loc.x as f64, self.geometry.loc.y as f64)),
            Size::from((self.geometry.size.w as f64, self.geometry.size.h as f64)),
        )
    }

    fn draw_gles(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let tex_size = self.texture.size();
        let uniforms = BlurShaders::finish_values(
            (tex_size.w as f32, tex_size.h as f32),
            (self.mask.loc.x as f32, self.mask.loc.y as f32),
            (self.mask.size.w as f32, self.mask.size.h as f32),
            self.radius,
            self.noise,
        );
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            opaque_regions,
            Transform::Normal,
            self.alpha,
            self.program.as_ref(),
            &uniforms,
        )
    }
}

impl Element for BlurBackdrop {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.source_rect()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        // The texture is regenerated wholesale, so any re-blur damages the
        // full backdrop; geometry changes are the damage tracker's job.
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry.size)])
        }
    }

    /// Empty even though the blur itself is opaque: the corners are cut away,
    /// and the backdrop fades with its window during animations.
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for BlurBackdrop {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        self.draw_gles(frame, src, dst, damage, opaque_regions)
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // Never a plane candidate: the pixels only exist through the shader.
        None
    }
}

impl<'render> RenderElement<KmsRenderer<'render>> for BlurBackdrop {
    fn draw(
        &self,
        frame: &mut MultiFrame<'render, 'render, '_, '_, GbmGlesApi, GbmGlesApi>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), MultiError<GbmGlesApi, GbmGlesApi>> {
        // The texture lives on the render device's GLES context — the same
        // one `frame.as_mut()` exposes — so this never crosses GPUs.
        self.draw_gles(frame.as_mut(), src, dst, damage, opaque_regions)
            .map_err(MultiError::Render)
    }

    fn underlying_storage(
        &self,
        _renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// The elements the blur pre-pass renders: exactly what the main pass draws
/// *under* the workspaces — Bottom and Background layer surfaces — imported
/// through the composition GPU's own GLES context.
pub fn source_elements(
    monitor: &Monitor,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<WaylandSurfaceRenderElement<GlesRenderer>> {
    use smithay::backend::renderer::element::AsRenderElements;

    let mut elements = Vec::new();
    let map = layer_map_for_output(monitor.output());
    for layer in [Layer::Bottom, Layer::Background] {
        for surface in map.layers_on(layer).rev() {
            let Some(geometry) = map.layer_geometry(surface) else {
                continue;
            };
            let location: Point<i32, Physical> = geometry.loc.to_physical_precise_round(scale);
            elements.extend(AsRenderElements::<GlesRenderer>::render_elements::<
                WaylandSurfaceRenderElement<GlesRenderer>,
            >(surface, renderer, location, scale, 1.0));
        }
    }
    elements
}

/// The `wl_surface` a window draws through, if it is a Wayland one.
///
/// X11 windows have no `wl_surface` of their own to hang a blur region on, so
/// they simply never blur.
pub fn window_surface(window: &Window) -> Option<WlSurface> {
    match window.underlying_surface() {
        WindowSurface::Wayland(toplevel) => Some(toplevel.wl_surface().clone()),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Whether a surface has a blur region committed at all.
pub fn has_blur_region(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        background_effect::blur_region(states).is_some()
    })
}

/// Whether anything visible on this output asked for blur.
///
/// The pre-pass is skipped entirely when nothing did, which is the common
/// case — so this scan runs every frame and is deliberately cheap.
pub fn output_wants_blur(shell: &Shell, monitor: &Monitor) -> bool {
    let windows = shell
        .visible_windows(monitor)
        .filter_map(|tile| window_surface(tile.window()))
        .any(|surface| has_blur_region(&surface));
    if windows {
        return true;
    }

    let map = layer_map_for_output(monitor.output());
    BLURRABLE_LAYERS.iter().any(|layer| {
        map.layers_on(*layer)
            .any(|layer_surface| has_blur_region(layer_surface.wl_surface()))
    })
}

/// Places a surface's committed blur region on the output, in `out`.
///
/// `origin` is where the surface's own `(0, 0)` lands in output-local physical
/// coordinates, and `clip` bounds the result. Returns the region's generation,
/// or `None` when the surface committed no region at all — which is not the
/// same as `out` coming back empty, since a client may legitimately set a
/// region that covers nothing.
pub fn place_blur_region(
    surface: &WlSurface,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    clip: Rectangle<i32, Physical>,
    out: &mut Vec<Rectangle<i32, Physical>>,
) -> Option<u32> {
    out.clear();

    let region = with_states(surface, background_effect::blur_region)?;

    // "The blur region is specified in the surface-local coordinates, and
    // clipped by the compositor to the surface size." The size is only known
    // once a buffer is attached; before that there is nothing to draw behind
    // anyway, so an absent size means an empty placement rather than an
    // unclipped one.
    let surface_size = with_renderer_surface_state(surface, |state| state.surface_size()).flatten();
    let Some(surface_size) = surface_size else {
        return Some(region.generation);
    };
    let surface_rect = Rectangle::from_size(surface_size);

    out.extend(
        region
            .rects
            .iter()
            .filter_map(|rect| place_rect(*rect, origin, scale, surface_rect, clip)),
    );

    Some(region.generation)
}

/// Places one surface-local rectangle on the output, or drops it if nothing of
/// it survives the surface and the clip.
fn place_rect(
    rect: Rectangle<i32, Logical>,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    surface: Rectangle<i32, Logical>,
    clip: Rectangle<i32, Physical>,
) -> Option<Rectangle<i32, Physical>> {
    let rect = rect.intersection(surface)?;

    // Rounded as extremities rather than as location-plus-size: at a
    // fractional scale the latter lets two rectangles that shared an edge in
    // logical space end up a pixel apart, and a seam through a translucent
    // backdrop is plainly visible.
    let placed = Rectangle::from_extremities(
        origin + rect.loc.to_physical_precise_round(scale),
        origin + (rect.loc + rect.size.to_point()).to_physical_precise_round(scale),
    );

    let placed = placed.intersection(clip)?;
    (!placed.is_empty()).then_some(placed)
}

/// Element identities and a commit counter for one surface's backdrops.
///
/// The ids have to be stable across frames or the damage tracker repaints the
/// surface's area every frame; the counter has to advance exactly when the
/// pixels change, which is when the texture underneath was re-blurred
/// (`serial`) or the client committed a different region (`generation`).
///
/// Both live on the surface, so a surface visited by two outputs in one frame
/// would make the counter oscillate. Nothing does that here: a window belongs
/// to one workspace on one monitor, and a layer surface to one output.
pub fn backdrop_slots(
    surface: &WlSurface,
    count: usize,
    serial: u64,
    generation: u32,
) -> (Vec<Id>, CommitCounter) {
    with_states(surface, |states| {
        let slots = states
            .data_map
            .get_or_insert_threadsafe(BackdropSlots::default);
        let mut slots = slots.0.lock().unwrap();

        if slots.seen != Some((serial, generation)) {
            slots.seen = Some((serial, generation));
            slots.commit.increment();
        }

        // Grown, never shrunk, so a region that gains and loses a rectangle
        // does not hand the same piece a new identity each time. The cap on
        // rectangles caps this too.
        while slots.ids.len() < count {
            slots.ids.push(Id::new());
        }

        (slots.ids[..count].to_vec(), slots.commit)
    })
}

#[derive(Debug, Default)]
struct BackdropSlots(Mutex<BackdropSlotsInner>);

#[derive(Debug, Default)]
struct BackdropSlotsInner {
    ids: Vec<Id>,
    commit: CommitCounter,
    seen: Option<(u64, u32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_clamps_passes() {
        let config = BlurConfig {
            passes: 0,
            ..Default::default()
        };
        assert_eq!(config.passes(), 1);
        let config = BlurConfig {
            passes: 40,
            ..Default::default()
        };
        assert_eq!(config.passes(), 8);
    }

    fn logical(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn physical(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_region_covering_the_surface_covers_the_whole_backdrop() {
        let placed = place_rect(
            logical(0, 0, 100, 50),
            (10, 20).into(),
            Scale::from(1.0),
            logical(0, 0, 100, 50),
            physical(10, 20, 100, 50),
        );
        assert_eq!(placed, Some(physical(10, 20, 100, 50)));
    }

    #[test]
    fn a_region_larger_than_the_surface_is_clipped_to_it() {
        // "The blur region is ... clipped by the compositor to the surface
        // size" — a client asking for more must not get more.
        let placed = place_rect(
            logical(-50, -50, 400, 400),
            (0, 0).into(),
            Scale::from(1.0),
            logical(0, 0, 100, 50),
            physical(0, 0, 1000, 1000),
        );
        assert_eq!(placed, Some(physical(0, 0, 100, 50)));
    }

    #[test]
    fn a_region_outside_the_surface_is_dropped() {
        let placed = place_rect(
            logical(200, 200, 10, 10),
            (0, 0).into(),
            Scale::from(1.0),
            logical(0, 0, 100, 50),
            physical(0, 0, 1000, 1000),
        );
        assert_eq!(placed, None);
    }

    #[test]
    fn a_region_outside_the_clip_is_dropped() {
        // A window mid-shrink still holds its old buffer; its blur must not
        // spill over the neighbour it is uncovering.
        let placed = place_rect(
            logical(0, 0, 100, 50),
            (0, 0).into(),
            Scale::from(1.0),
            logical(0, 0, 100, 50),
            physical(500, 500, 100, 50),
        );
        assert_eq!(placed, None);
    }

    #[test]
    fn adjacent_rectangles_stay_adjacent_at_a_fractional_scale() {
        // Rounding location and size separately would put a one-pixel seam
        // between these two, and a seam through translucent glass shows.
        let surface = logical(0, 0, 100, 100);
        let clip = physical(0, 0, 1000, 1000);
        let scale = Scale::from(1.25);

        for edge in 1..100 {
            let left = place_rect(logical(0, 0, edge, 10), (0, 0).into(), scale, surface, clip)
                .expect("left half is on screen");
            let right = place_rect(
                logical(edge, 0, 100 - edge, 10),
                (0, 0).into(),
                scale,
                surface,
                clip,
            )
            .expect("right half is on screen");

            assert_eq!(
                left.loc.x + left.size.w,
                right.loc.x,
                "seam at logical x={edge}"
            );
        }
    }

    #[test]
    fn fingerprint_tracks_the_knobs_that_invalidate_pixels() {
        let a = BlurConfig::default();
        let mut b = a;
        b.noise = 0.5;
        // Noise is applied at composite time, not baked into the texture, so
        // changing it must NOT force a re-blur.
        assert_eq!(a.fingerprint(), b.fingerprint());

        let mut c = a;
        c.offset = 3.0;
        assert_ne!(a.fingerprint(), c.fingerprint());
    }
}

//! GPU blur for `ext-background-effect-v1` surfaces.
//!
//! The model is Hyprland's cached pre-blur (its `new_optimizations` path):
//! once per output, everything that lives *under* the window stack — the
//! wallpaper and the Background/Bottom layer-shell surfaces — is rendered into
//! an offscreen texture and pushed through a dual-kawase pyramid. Each window
//! with a committed blur region then gets a [`BlurBackdrop`] element drawn
//! behind its surfaces, sampling the blurred texture at the window's on-screen
//! rectangle with the same rounded-corner mask the window is clipped with.
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
//! Everything here talks to a plain [`GlesRenderer`]; on the KMS backend that
//! is the render node's GLES context under the multi-GPU wrapper (the same
//! one the rounded-corner shader binds through), so the blurred texture lives
//! exactly where the output's composition happens.

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
            utils::{CommitCounter, DamageSet, OpaqueRegions},
        },
    },
    desktop::layer_map_for_output,
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::{compositor::with_states, shell::wlr_layer::Layer},
};

use crate::{
    backend::render::{GbmGlesApi, KmsRenderer},
    protocols::background_effect::BlurRegionCachedState,
    shaders::blur::BlurShaders,
    shell::monitor::Monitor,
};

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
    /// Bumped whenever `blurred` gets new content; backdrop elements report
    /// damage by comparing against it.
    commit: CommitCounter,
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
        self.commit.increment();
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
            commit: self.commit,
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
    commit: CommitCounter,
    noise: f32,
}

/// The blurred glass behind one window.
///
/// Samples [`BackdropSource`]'s screen-space texture at the window's
/// rectangle, so it holds physical coordinates directly — they were computed
/// against the same scale the texture was rendered at.
#[derive(Debug)]
pub struct BlurBackdrop {
    id: Id,
    commit: CommitCounter,
    texture: GlesTexture,
    /// `None` draws the sample unmasked (square corners, no dither) — the
    /// finish shader failing to compile must not drop the backdrop.
    program: Option<smithay::backend::renderer::gles::GlesTexProgram>,
    geometry: Rectangle<i32, Physical>,
    radius: f32,
    noise: f32,
    alpha: f32,
}

impl BlurBackdrop {
    pub fn new(
        renderer: &GlesRenderer,
        source: &BackdropSource,
        id: Id,
        geometry: Rectangle<i32, Physical>,
        radius: f32,
        alpha: f32,
    ) -> Self {
        let program = BlurShaders::get(renderer).map(|shaders| shaders.finish);
        Self {
            id,
            commit: source.commit,
            texture: source.texture.clone(),
            program,
            geometry,
            radius,
            noise: source.noise,
            alpha,
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
        let uniforms = BlurShaders::finish_values(
            (self.geometry.size.w as f32, self.geometry.size.h as f32),
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

/// The committed blur bounds of a window's surface, if any: the protocol's
/// region clipped to the surface, in surface-local logical coordinates.
pub fn window_blur_bounds(window: &smithay::desktop::Window) -> Option<Rectangle<i32, Logical>> {
    use smithay::desktop::WindowSurface;

    let surface = match window.underlying_surface() {
        WindowSurface::Wayland(toplevel) => toplevel.wl_surface().clone(),
        #[allow(unreachable_patterns)]
        _ => return None,
    };
    let surface_size = window.geometry().size;
    with_states(&surface, |states| {
        states
            .cached_state
            .get::<BlurRegionCachedState>()
            .current()
            .blur_bounds(surface_size)
    })
}

/// A stable render-element [`Id`] for a surface's backdrop, so the damage
/// tracker recognises it across frames instead of treating every frame's
/// backdrop as a brand-new element (which would repaint the window's area
/// every frame).
pub fn backdrop_id(window: &smithay::desktop::Window) -> Option<Id> {
    use smithay::desktop::WindowSurface;

    let surface = match window.underlying_surface() {
        WindowSurface::Wayland(toplevel) => toplevel.wl_surface().clone(),
        #[allow(unreachable_patterns)]
        _ => return None,
    };
    Some(with_states(&surface, |states| {
        states
            .data_map
            .insert_if_missing_threadsafe(|| BackdropId(Id::new()));
        states
            .data_map
            .get::<BackdropId>()
            .map(|id| id.0.clone())
            .unwrap_or_else(Id::new)
    }))
}

struct BackdropId(Id);

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

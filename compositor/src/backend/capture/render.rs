//! One capture frame: the output's scene rendered straight into a client
//! slot, or — for NV12 — into an RGB texture and converted on the GPU.

use smithay::{
    backend::{
        allocator::{Buffer, Fourcc, dmabuf::Dmabuf},
        renderer::{
            Bind, Renderer, Texture,
            damage::OutputDamageTracker,
            element::RenderElement,
            gles::{GlesRenderer, GlesTexture},
            sync::SyncPoint,
        },
    },
    input::pointer::CursorImageStatus,
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

use config::{Appearance, GlassSettings};
use protocols::crownos_screencast::{ClaimedSlot, CursorMode};

use crate::rendering::{corners::CornerMemory, warp::WarpCanvases};
use crate::{
    backend::{
        capture::{
            formats::buffer_constraints,
            nv12::Nv12Error,
            session::{CaptureSession, RenderTargets},
            source::{Placement, View},
        },
        render::{CrownRenderer, KmsRenderer},
    },
    rendering::{
        self, Elements, FrameStyle,
        blur::{BlurCache, BlurConfig, BlurSession},
        cursor::Cursor,
        decoration::TextRenderer,
        rounded::{GlesDecorator, MultiDecorator},
        sources::{CursorOverlay, window_elements, workspace_elements},
    },
    shaders::nv12::Nv12Shader,
    shell::{Shell, decoration::Control},
    utils::id::WindowId,
};

const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

/// What changed in a frame, and when the GPU is done drawing it.
pub type DrawnFrame = (Vec<Rectangle<i32, BufferCoords>>, SyncPoint);

/// A renderer a frame can be captured with.
pub trait CaptureRenderer: CrownRenderer + Bind<Dmabuf> + Bind<GlesTexture>
where
    Self::TextureId: Send + Clone + 'static,
{
    fn decorator<'a>(
        blur: Option<BlurSession<'a>>,
        corners: &'a mut CornerMemory,
        canvases: &'a mut WarpCanvases,
    ) -> Self::Decorator<'a>;

    /// The GLES renderer underneath, for the conversion passes.
    fn gles(&mut self) -> &mut GlesRenderer;
}

impl CaptureRenderer for GlesRenderer {
    fn decorator<'a>(
        blur: Option<BlurSession<'a>>,
        corners: &'a mut CornerMemory,
        canvases: &'a mut WarpCanvases,
    ) -> GlesDecorator<'a> {
        GlesDecorator::new(blur, corners, canvases)
    }

    fn gles(&mut self) -> &mut GlesRenderer {
        self
    }
}

impl<'render> CaptureRenderer for KmsRenderer<'render> {
    fn decorator<'a>(
        blur: Option<BlurSession<'a>>,
        corners: &'a mut CornerMemory,
        canvases: &'a mut WarpCanvases,
    ) -> MultiDecorator<'a> {
        MultiDecorator::new(blur, corners, canvases)
    }

    fn gles(&mut self) -> &mut GlesRenderer {
        self.as_mut()
    }
}

/// What the frame is drawn from.
pub struct Scene<'a> {
    pub shell: &'a Shell,
    pub placement: Placement<'a>,
    pub cursor: &'a mut Cursor,
    pub pointer: Point<f64, Logical>,
    pub appearance: &'a Appearance,
    pub glass: &'a GlassSettings,
    pub text: &'a mut TextRenderer,
    pub hovered: Option<(WindowId, Control)>,
}

pub struct RenderedFrame {
    pub index: usize,
    pub damage: Vec<Rectangle<i32, BufferCoords>>,
    pub sync: SyncPoint,
}

pub enum Outcome {
    /// Nothing to do: the session is not running, every slot is the
    /// client's, or nothing changed.
    Idle,
    /// New constraints were sent; frames resume once buffers match them.
    Reconfigured,
    Rendered(RenderedFrame),
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("failed to bind the capture target: {0}")]
    Bind(String),
    #[error("failed to render the capture frame: {0}")]
    Render(String),
    #[error(transparent)]
    Nv12(#[from] Nv12Error),
}

pub fn capture_frame<R>(
    renderer: &mut R,
    session: &mut CaptureSession,
    scene: Scene<'_>,
) -> Result<Outcome, CaptureError>
where
    R: CaptureRenderer,
    R::TextureId: Send + Clone + 'static,
{
    let scale = scene.placement.scale();
    let size: Size<i32, Physical> = scene.placement.area.size.to_physical_precise_round(scale);
    let buffer_size = Size::<i32, BufferCoords>::from((size.w, size.h));

    if session.protocol.constraint_size() != Some(buffer_size) || session.targets.damage.is_none() {
        let render_formats = Bind::<Dmabuf>::supported_formats(renderer).unwrap_or_default();
        let gpu_conversion = nv12_shader_ready(renderer.gles());
        session.protocol.set_constraints(buffer_constraints(
            &render_formats,
            buffer_size,
            gpu_conversion,
        ));
        session.targets.reset(size, scale, Transform::Normal);
        return Ok(Outcome::Reconfigured);
    }

    let Some(slot) = session.protocol.claim_slot() else {
        return Ok(Outcome::Idle);
    };
    let index = slot.index;
    let result = render_slot(
        renderer,
        &mut session.targets,
        session.protocol.cursor_mode(),
        slot,
        scene,
        size,
        scale,
    );
    if !matches!(result, Ok(Outcome::Rendered(_))) {
        session.protocol.abandon_slot(index);
    }
    result
}

fn render_slot<R>(
    renderer: &mut R,
    targets: &mut RenderTargets,
    cursor_mode: CursorMode,
    slot: ClaimedSlot,
    scene: Scene<'_>,
    size: Size<i32, Physical>,
    scale: f64,
) -> Result<Outcome, CaptureError>
where
    R: CaptureRenderer,
    R::TextureId: Send + Clone + 'static,
{
    let RenderTargets {
        damage,
        ages,
        blur,
        corners,
        canvases,
        nv12,
    } = targets;
    let Some(tracker) = damage.as_mut() else {
        return Ok(Outcome::Idle);
    };
    let effects = Effects {
        blur,
        corners,
        canvases,
    };
    let canvas = Canvas {
        size,
        scale,
        transform: Transform::Normal,
    };

    let target = slot.dmabuf.clone();
    let rendered = if target.format().code == Fourcc::Nv12 {
        let (mut intermediate, holds_previous) =
            nv12.intermediate(renderer.gles(), target.size())?;
        let age = usize::from(holds_previous && !slot.full_frame);
        let drawn = draw_scene(
            renderer,
            tracker,
            effects,
            cursor_mode,
            scene,
            canvas,
            DrawTarget::Texture(&mut intermediate),
            age,
        )?;
        match drawn {
            Some((damage, _)) => {
                let sync = nv12.convert(renderer.gles(), &intermediate, slot.index, &target)?;
                Some((damage, sync))
            }
            None => None,
        }
    } else {
        let age = if slot.full_frame {
            0
        } else {
            ages.age(slot.index)
        };
        draw_scene(
            renderer,
            tracker,
            effects,
            cursor_mode,
            scene,
            canvas,
            DrawTarget::Dmabuf(target),
            age,
        )?
    };

    let Some((damage, sync)) = rendered else {
        return Ok(Outcome::Idle);
    };
    ages.rendered(slot.index);
    Ok(Outcome::Rendered(RenderedFrame {
        index: slot.index,
        damage,
        sync,
    }))
}

/// The per-session effect caches a frame's decorations draw from.
pub struct Effects<'a> {
    pub blur: &'a mut BlurCache,
    pub corners: &'a mut CornerMemory,
    pub canvases: &'a mut WarpCanvases,
}

/// The image being drawn: its size upright in pixels, its scale, and how it
/// is turned to fit the buffer.
#[derive(Clone, Copy)]
pub struct Canvas {
    pub size: Size<i32, Physical>,
    pub scale: f64,
    pub transform: Transform,
}

pub enum DrawTarget<'a> {
    Dmabuf(Dmabuf),
    Texture(&'a mut GlesTexture),
}

/// Draws `scene` into `target`, whose contents are `age` frames old, and
/// returns the damage, or `None` when nothing needed drawing.
#[allow(clippy::too_many_arguments)]
pub fn draw_scene<R>(
    renderer: &mut R,
    tracker: &mut OutputDamageTracker,
    effects: Effects<'_>,
    cursor_mode: CursorMode,
    scene: Scene<'_>,
    canvas: Canvas,
    target: DrawTarget<'_>,
    age: usize,
) -> Result<Option<DrawnFrame>, CaptureError>
where
    R: CaptureRenderer,
    R::TextureId: Send + Clone + 'static,
{
    let Effects {
        blur,
        corners,
        canvases,
    } = effects;
    let Canvas {
        size,
        scale,
        transform,
    } = canvas;
    blur.begin_frame();
    corners.begin_frame();
    canvases.begin_frame();
    let content = blur.content();
    let blur_config = BlurConfig::new(scene.appearance, scene.glass);
    let blur_session = blur_config.enabled.then_some(BlurSession {
        cache: &mut *blur,
        config: blur_config,
        transform,
        output: Rectangle::from_size(size),
    });
    let mut decorator = R::decorator(blur_session, corners, canvases);
    let elements = scene_elements(renderer, &mut decorator, cursor_mode, scene, scale);
    content.observe(
        || {
            OutputDamageTracker::new(
                transform.invert().transform_size(size),
                Scale::from(scale),
                transform,
            )
        },
        &elements,
    );
    match target {
        DrawTarget::Dmabuf(mut dmabuf) => {
            render_into(renderer, tracker, &mut dmabuf, age, &elements)
        }
        DrawTarget::Texture(texture) => render_into(renderer, tracker, texture, age, &elements),
    }
}

/// The scene, with the cursor left out unless it is to be embedded.
fn scene_elements<'frame, R>(
    renderer: &mut R,
    decorator: &mut R::Decorator<'frame>,
    cursor_mode: CursorMode,
    scene: Scene<'_>,
    scale: f64,
) -> Elements<R, R::Decorator<'frame>>
where
    R: CaptureRenderer,
    R::TextureId: Send + Clone + 'static,
{
    let Scene {
        shell,
        placement,
        cursor,
        pointer,
        appearance,
        glass: _,
        text,
        hovered,
    } = scene;

    let monitor = placement.monitor;
    let mut style = FrameStyle::new(
        appearance,
        scale,
        monitor.geometry().loc,
        text,
        shell.focused_window_id(),
        hovered,
    );
    let scale = Scale::from(scale);

    let embedded = cursor_mode == CursorMode::Embedded;
    let overlay = |cursor| {
        (embedded && placement.on_screen).then_some(CursorOverlay {
            cursor,
            pointer,
            origin: placement.area.loc,
        })
    };
    // A window or a workspace is captured off screen; while locked it is
    // nothing at all, or a recording would carry on past the lock.
    let locked = shell.is_locked() && !matches!(placement.view, View::Output);
    if locked {
        return Elements::<R, R::Decorator<'frame>>::new();
    }
    match placement.view {
        View::Window(tile) => window_elements(
            shell,
            tile,
            renderer,
            decorator,
            overlay(cursor),
            scale,
            &mut style,
        ),
        View::Workspace(workspace) => workspace_elements(
            shell,
            monitor,
            workspace,
            renderer,
            decorator,
            overlay(cursor),
            scale,
            &mut style,
        ),
        View::Output => {
            let status = (!embedded)
                .then(|| std::mem::replace(&mut cursor.status, CursorImageStatus::Hidden));
            let elements = rendering::output_elements(
                shell, monitor, renderer, decorator, cursor, pointer, scale, &mut style,
            );
            if let Some(status) = status {
                cursor.status = status;
            }
            elements
        }
    }
}

/// Renders into `target`, returning the damage in buffer coordinates, or
/// `None` when nothing needed drawing.
fn render_into<R, T, E>(
    renderer: &mut R,
    damage_tracker: &mut OutputDamageTracker,
    target: &mut T,
    age: usize,
    elements: &[E],
) -> Result<Option<DrawnFrame>, CaptureError>
where
    R: Renderer + Bind<T>,
    R::TextureId: Texture,
    E: RenderElement<R>,
{
    let mut framebuffer = renderer
        .bind(target)
        .map_err(|err| CaptureError::Bind(err.to_string()))?;
    let result = damage_tracker
        .render_output(renderer, &mut framebuffer, age, elements, CLEAR_COLOR)
        .map_err(|err| CaptureError::Render(format!("{err:?}")))?;

    let Some(damage) = result.damage.filter(|damage| !damage.is_empty()) else {
        return Ok(None);
    };
    let damage = damage
        .iter()
        .map(|rect| {
            Rectangle::new(
                (rect.loc.x, rect.loc.y).into(),
                (rect.size.w, rect.size.h).into(),
            )
        })
        .collect();
    Ok(Some((damage, result.sync)))
}

/// Whether NV12 can be offered: the conversion program compiled, now or
/// earlier on this GPU.
fn nv12_shader_ready(gles: &mut GlesRenderer) -> bool {
    if Nv12Shader::get(gles).is_some() {
        return true;
    }
    match Nv12Shader::init(gles) {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(%err, "the NV12 conversion shader failed to compile; capture offers RGB only");
            false
        }
    }
}

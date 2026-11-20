//! A frame of the nested window.
//!
//! One viewport draws straight into the window. A split window draws each
//! dirty viewport into its own texture, then composites the textures into
//! the window; the area of a viewport that is not connected stays dark grey.

use anyhow::{Context, anyhow};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Offscreen,
            damage::OutputDamageTracker,
            element::{
                Kind,
                texture::{TextureRenderBuffer, TextureRenderElement},
            },
            gles::{GlesRenderer, GlesTarget, GlesTexture},
        },
    },
    utils::{
        Buffer, Clock as MonotonicClock, Logical, Monotonic, Physical, Point, Rectangle, Scale,
        Size, Transform,
    },
};

use crate::{
    backend::{
        capture::CaptureState,
        debug_frames,
        present::{self, ReleasedClients},
        winit::{WinitState, viewport::Viewport},
    },
    rendering::{
        self, FrameStyle,
        blur::{BlurConfig, BlurSession},
        cursor::Cursor,
        decoration::TextRenderer,
        rounded::GlesDecorator,
    },
    shell::{Shell, decoration::Control},
    state::State,
    utils::id::WindowId,
};

const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];
const DISCONNECTED_COLOR: [f32; 4] = [0.2, 0.2, 0.2, 1.0];

type Damage = Option<Vec<Rectangle<i32, Physical>>>;

/// What every viewport of one frame draws from.
struct Scene<'frame> {
    shell: &'frame mut Shell,
    cursor: &'frame mut Cursor,
    pointer: Point<f64, Logical>,
    appearance: &'frame config::Appearance,
    glass: &'frame config::GlassSettings,
    text: &'frame mut TextRenderer,
    hovered: Option<(WindowId, Control)>,
}

/// Draws the window. Returns the clients whose FIFO barriers it released.
pub fn render(state: &mut State) -> anyhow::Result<ReleasedClients> {
    let mut released = ReleasedClients::default();

    debug_frames::script(state);
    // Rendered inside a dispatch, so whatever that dispatch changed is not
    // laid out by the idle hook yet.
    state.layout_menus();

    let State {
        backend,
        common,
        shell,
        clock,
        config,
        input,
        text,
        capture,
        ..
    } = state;

    let Some(winit) = backend.winit() else {
        return Ok(released);
    };

    let dt = clock.tick();
    let dt = debug_frames::fixed_dt().unwrap_or(dt);
    shell.advance_animations(dt);
    shell
        .surface_animations
        .advance(MonotonicClock::<Monotonic>::new().now().into());
    let animating = shell.is_animating();
    if !animating {
        // Land the last frame on exact integers rather than resting a fraction
        // of a pixel off, where the spring's epsilon stopped it.
        shell.settle_animations();
    }
    let spinning = animating || std::env::var_os("CROWN_DEBUG_SPIN").is_some();
    if spinning {
        winit.mark_all_dirty();
    }

    let mut scene = Scene {
        shell,
        cursor: &mut input.cursor,
        pointer: input.pointer_location,
        appearance: &config.current.appearance,
        glass: &config.current.glass,
        text,
        hovered: input.hovered_control,
    };

    let drawn = if winit.is_split() {
        render_split(winit, &mut scene, capture)?
    } else {
        render_single(winit, &mut scene, capture)?
    };

    if let Some(first) = winit
        .viewports
        .first()
        .filter(|viewport| viewport.is_connected())
        && debug_frames::compare(
            (
                &*scene.shell,
                &mut *scene.cursor,
                scene.pointer,
                scene.appearance,
                scene.glass,
                &mut *scene.text,
                scene.hovered,
            ),
            winit.backend.renderer(),
            &first.output,
        )
        && let Ok((renderer, framebuffer)) = winit.backend.bind()
    {
        debug_frames::rebind(renderer, &framebuffer);
    }

    let now = common.start_time.elapsed();
    for viewport in drawn.iter().filter_map(|index| winit.viewports.get(*index)) {
        present::send_frame_callbacks(scene.shell, &viewport.output, scene.cursor, now);
        present::signal_fifo_barriers(scene.shell, &viewport.output, scene.cursor, &mut released);
    }

    // Winit only redraws on demand. Scheduling only while something is moving is
    // what takes an idle desktop from a permanent 60 Hz loop to ~0% CPU; a client
    // that damages its surface wakes us through its own commit.
    if spinning {
        winit.backend.window().request_redraw();
    }

    Ok(released)
}

/// The one viewport, straight into the window. Returns the viewports drawn.
fn render_single(
    winit: &mut WinitState,
    scene: &mut Scene<'_>,
    capture: &mut CaptureState,
) -> anyhow::Result<Vec<usize>> {
    // A hardcoded age of 0 makes every frame a full repaint.
    let age = winit.backend.buffer_age().unwrap_or(0);
    let Some(viewport) = winit.viewports.first_mut() else {
        return Ok(Vec::new());
    };
    if !viewport.is_connected() {
        let damage = {
            let (renderer, mut framebuffer) = winit
                .backend
                .bind()
                .map_err(|err| anyhow!("Failed to bind the winit framebuffer: {err}"))?;
            let nothing: [TextureRenderElement<GlesTexture>; 0] = [];
            viewport
                .damage_tracker
                .render_output(renderer, &mut framebuffer, 0, &nothing, DISCONNECTED_COLOR)
                .with_context(|| "Failed to clear the nested window")?
                .damage
                .cloned()
        };
        winit
            .backend
            .submit(damage.as_deref())
            .map_err(|err| anyhow!("Failed to submit the winit frame: {err}"))?;
        return Ok(Vec::new());
    }
    let damage = {
        let (renderer, mut framebuffer) = winit
            .backend
            .bind()
            .map_err(|err| anyhow!("Failed to bind the winit framebuffer: {err}"))?;
        let damage = draw_viewport(renderer, &mut framebuffer, age, viewport, scene)?;
        if damage.as_ref().is_some_and(|damage| !damage.is_empty()) {
            debug_frames::grab(
                renderer,
                &framebuffer,
                &viewport.output,
                damage.as_deref(),
                age,
            );
            capture.mark_output_damaged(&viewport.output);
        }
        damage
    };
    winit
        .backend
        .submit(damage.as_deref())
        .map_err(|err| anyhow!("Failed to submit the winit frame: {err}"))?;
    Ok(vec![0])
}

/// Each dirty viewport into its texture, then every texture into the window.
fn render_split(
    winit: &mut WinitState,
    scene: &mut Scene<'_>,
    capture: &mut CaptureState,
) -> anyhow::Result<Vec<usize>> {
    let age = winit.window_buffer_age;
    let mut drawn = Vec::new();
    for (index, viewport) in winit.viewports.iter_mut().enumerate() {
        if !viewport.dirty || !viewport.is_connected() {
            continue;
        }
        let damaged = draw_into_target(winit.backend.renderer(), viewport, scene, index == 0)?;
        if damaged {
            capture.mark_output_damaged(&viewport.output);
        }
        viewport.dirty = false;
        drawn.push(index);
    }

    let damage = {
        let (renderer, mut framebuffer) = winit
            .backend
            .bind()
            .map_err(|err| anyhow!("Failed to bind the winit framebuffer: {err}"))?;
        let elements: Vec<TextureRenderElement<GlesTexture>> = winit
            .viewports
            .iter()
            .filter(|viewport| viewport.is_connected())
            .filter_map(|viewport| {
                viewport.target.as_ref().map(|target| {
                    TextureRenderElement::from_texture_render_buffer(
                        viewport.rect.loc.to_f64(),
                        target,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    )
                })
            })
            .collect();
        let tracker = &mut winit.window_damage_tracker;
        let mut composite = |age| {
            tracker
                .render_output(
                    renderer,
                    &mut framebuffer,
                    age,
                    &elements,
                    DISCONNECTED_COLOR,
                )
                .map(|result| result.damage.cloned())
                .with_context(|| "Failed to composite the nested outputs")
        };
        // The swap paces the next frame, and only drawing makes the surface
        // current for it, so a frame with nothing new still draws it all.
        match composite(age)? {
            Some(damage) if !damage.is_empty() => Some(damage),
            _ => composite(0)?,
        }
    };
    winit
        .backend
        .submit(damage.as_deref())
        .map_err(|err| anyhow!("Failed to submit the winit frame: {err}"))?;
    // Read now, while the swap left the surface current: drawing the
    // viewports into their textures next frame makes it not current.
    winit.window_buffer_age = winit.backend.buffer_age().unwrap_or(0);
    Ok(drawn)
}

/// Draws a viewport into its texture. Returns whether anything changed.
fn draw_into_target(
    renderer: &mut GlesRenderer,
    viewport: &mut Viewport,
    scene: &mut Scene<'_>,
    debugged: bool,
) -> anyhow::Result<bool> {
    let fresh = viewport.target.is_none();
    let mut target = match viewport.target.take() {
        Some(target) => target,
        None => create_target(renderer, viewport.rect.size)?,
    };
    let age = if fresh { 0 } else { 1 };
    let mut damaged = false;
    target
        .render()
        .draw(|texture| -> anyhow::Result<Vec<Rectangle<i32, Buffer>>> {
            let mut framebuffer = renderer
                .bind(texture)
                .map_err(|err| anyhow!("Failed to bind a nested output's texture: {err}"))?;
            let damage = draw_viewport(renderer, &mut framebuffer, age, viewport, scene)?;
            let damage = damage.unwrap_or_default();
            damaged = !damage.is_empty();
            if damaged && debugged {
                debug_frames::grab(renderer, &framebuffer, &viewport.output, Some(&damage), age);
            }
            Ok(damage
                .iter()
                .map(|rect| {
                    Rectangle::new(
                        (rect.loc.x, rect.loc.y).into(),
                        (rect.size.w, rect.size.h).into(),
                    )
                })
                .collect())
        })?;
    viewport.target = Some(target);
    Ok(damaged)
}

fn create_target(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> anyhow::Result<TextureRenderBuffer<GlesTexture>> {
    let size: Size<i32, Buffer> = (size.w, size.h).into();
    let texture = Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, size)
        .map_err(|err| anyhow!("Failed to create a nested output's texture: {err}"))?;
    Ok(TextureRenderBuffer::from_texture(
        renderer,
        texture,
        1,
        Transform::Normal,
        Some(vec![Rectangle::from_size(size)]),
    ))
}

/// One viewport's scene into `framebuffer`, whose contents are `age` frames old.
fn draw_viewport(
    renderer: &mut GlesRenderer,
    framebuffer: &mut GlesTarget<'_>,
    age: usize,
    viewport: &mut Viewport,
    scene: &mut Scene<'_>,
) -> anyhow::Result<Damage> {
    let Some(monitor) = scene.shell.monitor(&viewport.output) else {
        return Ok(None);
    };
    let scale = Scale::from(viewport.output.current_scale().fractional_scale());

    // Backdrops blur the framebuffer as they are drawn, so all the frame
    // owes them is the pyramids they cached last time and the bounds to
    // clip to.
    let bounds: Rectangle<i32, Physical> =
        Rectangle::from_size(monitor.geometry().size.to_physical_precise_round(scale));
    viewport.blur.begin_frame();
    viewport.corners.begin_frame();
    viewport.canvases.begin_frame();
    let content = viewport.blur.content();
    let blur_config = BlurConfig::new(scene.appearance, scene.glass);
    let blur_config = if scene.shell.lock_screen.is_active() {
        blur_config.for_lock()
    } else {
        blur_config
    };
    let blur = blur_config.enabled.then_some(BlurSession {
        cache: &mut viewport.blur,
        config: blur_config,
        transform: viewport.output.current_transform(),
        output: bounds,
    });

    let elements = rendering::output_elements(
        scene.shell,
        monitor,
        renderer,
        &mut GlesDecorator::new(blur, &mut viewport.corners, &mut viewport.canvases),
        scene.cursor,
        scene.pointer,
        scale,
        &mut FrameStyle::new(
            scene.appearance,
            scale.y,
            monitor.geometry().loc,
            scene.text,
            scene.shell.focused_window_id(),
            scene.hovered,
        ),
    );

    content.observe(
        || OutputDamageTracker::from_output(&viewport.output),
        &elements,
    );
    let result = viewport
        .damage_tracker
        .render_output(renderer, framebuffer, age, &elements, CLEAR_COLOR)
        .with_context(|| "Failed to render the output")?;

    present::track_primary_scanout(scene.shell, &viewport.output, scene.cursor, &result.states);
    if scene.shell.is_locked() {
        scene.shell.output_drew_locked(&viewport.output);
    }
    Ok(result.damage.cloned())
}

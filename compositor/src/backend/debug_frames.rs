//! TEMPORARY instrumentation: diffs every incremental frame against a full
//! repaint rendered offscreen, to find where partial damage and the blur
//! disagree. Enabled by CROWN_DEBUG_FRAMES=<dir>.

use std::{cell::RefCell, path::PathBuf, time::Instant};

use smithay::backend::renderer::Texture as _;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Blit, ExportMem, Offscreen, TextureFilter, TextureMapping,
            damage::OutputDamageTracker,
            gles::{GlesRenderer, GlesTarget, GlesTexture},
        },
    },
    output::Output,
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size},
};

use crate::{
    rendering::{
        self, FrameStyle,
        blur::{BlurCache, BlurConfig, BlurSession},
        rounded::GlesDecorator,
    },
    state::State,
};

struct Debug {
    dir: PathBuf,
    frame: u32,
    dumps: u32,
    tracker: OutputDamageTracker,
    blur: BlurCache,
    texture: Option<GlesTexture>,
    started: Instant,
    script: Vec<(f64, String)>,
    pending: Option<(Vec<(i32, i32, i32, i32)>, usize)>,
    grabbed: Option<GlesTexture>,
}

thread_local! {
    static DEBUG: RefCell<Option<Debug>> = const { RefCell::new(None) };
}

pub fn script(state: &mut State) {
    DEBUG.with_borrow_mut(|harness| {
        let Some(harness) = harness else { return };
        let elapsed = harness.started.elapsed().as_secs_f64();
        while harness.script.first().is_some_and(|(at, _)| *at <= elapsed) {
            let (_, action) = harness.script.remove(0);
            tracing::info!(%action, "debug script");
            if action == "overview"
                && let Some(monitor) = state.shell.focused_monitor_mut()
            {
                monitor.with_spacecontrol(|space, monitor| space.toggle(monitor));
            }
            let mut parts = action.split('@');
            if parts.next() == Some("click")
                && let (Some(Ok(x)), Some(Ok(y))) = (
                    parts.next().map(str::parse::<f64>),
                    parts.next().map(str::parse::<f64>),
                )
            {
                use protocols::crownos_input::{ButtonState, InjectedEvent, InjectedFrame};
                let time = smithay::backend::input::InputTime::from_micros(
                    (elapsed * 1e6) as u64,
                );
                state.warp_pointer((x, y).into(), time);
                for button_state in [ButtonState::Pressed, ButtonState::Released] {
                    state.apply_injected_frame(InjectedFrame {
                        events: vec![InjectedEvent::Button {
                            button: 0x110,
                            state: button_state,
                        }],
                        time_usec: (elapsed * 1e6) as u64,
                    });
                }
            }
            if let Some(rest) = action.strip_prefix("move@")
                && let Some((x, y)) = rest.split_once('@')
                && let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>())
            {
                let time = smithay::backend::input::InputTime::from_micros(
                    (elapsed * 1e6) as u64,
                );
                state.warp_pointer((x, y).into(), time);
            }
        }
    });
}

pub fn init(output: &Output) {
    let Ok(dir) = std::env::var("CROWN_DEBUG_FRAMES") else {
        return;
    };
    let script = std::env::var("CROWN_DEBUG_SCRIPT")
        .unwrap_or_default()
        .split(',')
        .filter_map(|item| {
            let (at, action) = item.split_once(':')?;
            Some((at.parse().ok()?, action.to_owned()))
        })
        .collect();
    DEBUG.with_borrow_mut(|harness| {
        *harness = Some(Debug {
            dir: dir.into(),
            frame: 0,
            dumps: 0,
            tracker: OutputDamageTracker::from_output(output),
            blur: BlurCache::default(),
            texture: None,
            started: Instant::now(),
            script,
            pending: None,
            grabbed: None,
        })
    });
}

fn read(
    renderer: &mut GlesRenderer,
    target: &GlesTarget<'_>,
    size: Size<i32, BufferCoords>,
) -> Option<(Vec<u8>, bool)> {
    let mapping = renderer
        .copy_framebuffer(target, Rectangle::from_size(size), Fourcc::Abgr8888)
        .ok()?;
    let flipped = mapping.flipped();
    Some((renderer.map_texture(&mapping).ok()?.to_vec(), flipped))
}

fn write_ppm(path: &PathBuf, pixels: &[u8], size: Size<i32, BufferCoords>, flipped: bool) {
    let (w, h) = (size.w as usize, size.h as usize);
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for row in 0..h {
        let row = if flipped { h - 1 - row } else { row };
        for px in pixels[row * w * 4..(row + 1) * w * 4].chunks_exact(4) {
            out.extend_from_slice(&px[..3]);
        }
    }
    let _ = std::fs::write(path, out);
}

pub fn grab(
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    output: &Output,
    damage: Option<&[Rectangle<i32, Physical>]>,
    age: usize,
) {
    DEBUG.with_borrow_mut(|harness| {
        let Some(harness) = harness else { return };
        let Some(mode) = output.current_mode() else {
            return;
        };
        let size: Size<i32, BufferCoords> = (mode.size.w, mode.size.h).into();
        let mut grabbed = match harness.grabbed.take().filter(|texture| texture.size() == size) {
            Some(texture) => texture,
            None => match Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, size)
            {
                Ok(texture) => texture,
                Err(_) => return,
            },
        };
        {
            let Ok(mut target) = renderer.bind(&mut grabbed) else {
                return;
            };
            let rect = Rectangle::from_size((size.w, size.h).into());
            if let Err(err) =
                renderer.blit(framebuffer, &mut target, rect, rect, TextureFilter::Nearest)
            {
                tracing::warn!(%err, "debug: grab blit failed");
                return;
            }
        }
        harness.grabbed = Some(grabbed);
        let damage = damage
            .unwrap_or_default()
            .iter()
            .map(|r| (r.loc.x, r.loc.y, r.size.w, r.size.h))
            .collect();
        harness.pending = Some((damage, age));
    });
}

pub fn compare(
    state_parts: (
        &crate::shell::Shell,
        &mut crate::rendering::cursor::Cursor,
        smithay::utils::Point<f64, smithay::utils::Logical>,
        &config::Appearance,
        &mut crate::rendering::decoration::TextRenderer,
        Option<(crate::utils::id::WindowId, crate::shell::decoration::Control)>,
    ),
    renderer: &mut GlesRenderer,
    output: &Output,
) -> bool {
    DEBUG.with_borrow_mut(|harness| {
        let Some(harness) = harness else { return false };
        let Some((damage_summary, age)) = harness.pending.take() else {
            return false;
        };
        let Some(mut grabbed) = harness.grabbed.take() else {
            return true;
        };
        let size_early: Size<i32, BufferCoords> = grabbed.size();
        let incremental = {
            let Ok(target) = renderer.bind(&mut grabbed) else {
                return true;
            };
            read(renderer, &target, size_early)
        };
        harness.grabbed = Some(grabbed);
        let Some((incremental, flipped)) = incremental else {
            return true;
        };
        let (shell, cursor, pointer, appearance, text, hovered) = state_parts;
        let Some(mode) = output.current_mode() else {
            return true;
        };
        let size: Size<i32, BufferCoords> = (mode.size.w, mode.size.h).into();
        let scale = smithay::utils::Scale::from(output.current_scale().fractional_scale());
        let Some(monitor) = shell.monitor(output) else {
            return true;
        };
        harness.frame += 1;

        let mut texture = match harness.texture.take().filter(|texture| texture.size() == size) {
            Some(texture) => texture,
            None => match Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, size)
            {
                Ok(texture) => texture,
                Err(err) => {
                    tracing::warn!(%err, "debug: offscreen failed");
                    return true;
                }
            },
        };

        let reference = {
            let Ok(mut target) = renderer.bind(&mut texture) else {
                return true;
            };
            let bounds: Rectangle<i32, Physical> =
                Rectangle::from_size(monitor.geometry().size.to_physical_precise_round(scale));
            harness.blur.begin_frame();
            let config = BlurConfig::from(appearance);
            let blur = config.enabled.then_some(BlurSession {
                cache: &mut harness.blur,
                config,
                transform: output.current_transform(),
                output: bounds,
            });
            let elements = rendering::output_elements(
                shell,
                monitor,
                renderer,
                &mut GlesDecorator::new(blur),
                cursor,
                pointer,
                scale,
                &mut FrameStyle::new(
                    appearance,
                    scale.y,
                    monitor.geometry().loc,
                    text,
                    shell.focused_window_id(),
                    hovered,
                ),
            );
            if let Err(err) =
                harness
                    .tracker
                    .render_output(renderer, &mut target, 0, &elements, [0.1, 0.1, 0.1, 1.0])
            {
                tracing::warn!(%err, "debug: reference render failed");
                return true;
            }
            read(renderer, &target, size)
        };
        let Some((reference, reference_flipped)) = reference else {
            return true;
        };
        harness.texture = Some(texture);

        let (w, h) = (size.w as usize, size.h as usize);
        let mut count = 0usize;
        let mut worst = 0u8;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..h {
            let ry = if flipped == reference_flipped { y } else { h - 1 - y };
            for x in 0..w {
                let a = &incremental[(y * w + x) * 4..(y * w + x) * 4 + 3];
                let b = &reference[(ry * w + x) * 4..(ry * w + x) * 4 + 3];
                let diff = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                if diff > std::env::var("CROWN_DEBUG_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(24u8) {
                    count += 1;
                    worst = worst.max(diff);
                    let sy = if flipped { h - 1 - y } else { y };
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(sy);
                    y1 = y1.max(sy);
                }
            }
        }

        tracing::info!(
            frame = harness.frame,
            age,
            count,
            worst,
            bbox = ?(x0, y0, x1, y1),
            damage = ?damage_summary,
            "debug diff"
        );

        let snap = harness.dir.join("snap");
        let requested = std::fs::remove_file(&snap).is_ok();
        if requested || (count > 200 && harness.dumps < 40) {
            harness.dumps += 1;
            let base = harness.dir.join(format!("f{:05}", harness.frame));
            write_ppm(&base.with_extension("inc.ppm"), &incremental, size, flipped);
            write_ppm(&base.with_extension("ref.ppm"), &reference, size, reference_flipped);
        }
        true
    })
}

pub fn rebind(renderer: &mut GlesRenderer, framebuffer: &GlesTarget<'_>) {
    DEBUG.with_borrow_mut(|harness| {
        let Some(harness) = harness else { return };
        let Some(grabbed) = harness.grabbed.as_mut() else {
            return;
        };
        let Ok(mut target) = renderer.bind(grabbed) else {
            return;
        };
        let rect = Rectangle::from_size((1, 1).into());
        let _ = renderer.blit(framebuffer, &mut target, rect, rect, TextureFilter::Nearest);
    });
}

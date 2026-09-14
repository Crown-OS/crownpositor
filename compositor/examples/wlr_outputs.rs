//! A minimal `zwlr_output_management_v1` client, for exercising the
//! compositor's own implementation.
//!
//! `wlr-randr` is the usual tool for this, but it is not always installed and
//! it hides the protocol behind its own opinions. This prints exactly what the
//! compositor sent, and can push a configuration back.
//!
//! ```text
//! cargo run -p compositor --example wlr_outputs               # list
//! cargo run -p compositor --example wlr_outputs -- --test  DP-1 scale=1.5
//! cargo run -p compositor --example wlr_outputs -- --apply DP-1 scale=1.5 pos=0,0
//! ```
//!
//! Every head must be named in a configuration — the protocol says omitting
//! one is an error — so heads left off the command line are re-sent with the
//! values they already have.

use std::collections::HashMap;

use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry},
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

#[derive(Debug, Default, Clone)]
struct Mode {
    width: i32,
    height: i32,
    refresh: i32,
    preferred: bool,
}

#[derive(Debug, Default)]
struct Head {
    name: String,
    description: String,
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    physical: (i32, i32),
    modes: Vec<(ZwlrOutputModeV1, Mode)>,
    current_mode: Option<ZwlrOutputModeV1>,
    enabled: bool,
    position: (i32, i32),
    transform: Option<wl_output::Transform>,
    scale: f64,
    adaptive_sync: Option<bool>,
}

#[derive(Default)]
struct App {
    heads: HashMap<ZwlrOutputHeadV1, Head>,
    order: Vec<ZwlrOutputHeadV1>,
    serial: Option<u32>,
    outcome: Option<&'static str>,
}

fn main() {
    let connection = Connection::connect_to_env().expect("no Wayland compositor to talk to");
    let (globals, mut queue) =
        registry_queue_init::<App>(&connection).expect("the registry is unreadable");
    let qh = queue.handle();

    let manager: ZwlrOutputManagerV1 = globals
        .bind(&qh, 1..=4, ())
        .expect("the compositor does not implement zwlr_output_management_v1");

    let mut app = App::default();
    // The first `done` means the head list is complete.
    while app.serial.is_none() {
        queue.blocking_dispatch(&mut app).expect("dispatch failed");
    }

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.split_first() {
        Some((flag, rest)) if flag == "--apply" || flag == "--test" => {
            configure(&mut app, &mut queue, &qh, &manager, flag == "--test", rest);
        }
        _ => print_heads(&app),
    }
}

fn print_heads(app: &App) {
    println!("serial {}", app.serial.unwrap_or_default());
    for handle in &app.order {
        let Some(head) = app.heads.get(handle) else {
            continue;
        };
        println!("\n{} ({})", head.name, head.description);
        println!("  enabled:  {}", head.enabled);
        println!(
            "  make/model/serial: {}/{}/{}",
            head.make.as_deref().unwrap_or("-"),
            head.model.as_deref().unwrap_or("-"),
            head.serial.as_deref().unwrap_or("-")
        );
        println!("  physical: {}x{} mm", head.physical.0, head.physical.1);
        println!("  position: {},{}", head.position.0, head.position.1);
        println!("  scale:    {}", head.scale);
        println!("  transform: {:?}", head.transform);
        println!("  adaptive sync: {:?}", head.adaptive_sync);
        println!("  modes:");
        for (handle, mode) in &head.modes {
            let current = head.current_mode.as_ref() == Some(handle);
            println!(
                "    {}x{}@{:.3}{}{}",
                mode.width,
                mode.height,
                mode.refresh as f64 / 1000.0,
                if mode.preferred { " preferred" } else { "" },
                if current { " *current*" } else { "" },
            );
        }
    }
}

/// `NAME key=value ...`, where the keys are `scale`, `pos`, `transform`,
/// `mode` (`WxH` or `WxH@Hz`), `vrr` and `off`.
fn configure(
    app: &mut App,
    queue: &mut wayland_client::EventQueue<App>,
    qh: &QueueHandle<App>,
    manager: &ZwlrOutputManagerV1,
    test_only: bool,
    arguments: &[String],
) {
    let mut wanted: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<String> = None;
    for argument in arguments {
        if argument.contains('=') {
            if let Some(name) = &current {
                wanted.entry(name.clone()).or_default().push(argument.clone());
            }
        } else {
            current = Some(argument.clone());
            wanted.entry(argument.clone()).or_default();
        }
    }

    let serial = app.serial.expect("a serial arrives before any configuration");
    let configuration = manager.create_configuration(serial, qh, ());

    for handle in app.order.clone() {
        let Some(head) = app.heads.get(&handle) else {
            continue;
        };
        let settings = wanted.get(&head.name);
        let off = settings.is_some_and(|settings| settings.iter().any(|s| s == "off=true"));

        if off {
            configuration.disable_head(&handle);
            continue;
        }

        let configured = configuration.enable_head(&handle, qh, ());
        let Some(settings) = settings else {
            continue;
        };

        for setting in settings {
            let Some((key, value)) = setting.split_once('=') else {
                continue;
            };
            match key {
                "scale" => configured.set_scale(value.parse().unwrap_or(1.0)),
                "pos" => {
                    if let Some((x, y)) = value.split_once(',') {
                        configured
                            .set_position(x.parse().unwrap_or(0), y.parse().unwrap_or(0));
                    }
                }
                "transform" => {
                    if let Some(transform) = parse_transform(value) {
                        configured.set_transform(transform);
                    }
                }
                "mode" => {
                    if let Some(mode) = find_mode(head, value) {
                        configured.set_mode(&mode);
                    } else {
                        eprintln!("{}: no mode matching {value}", head.name);
                    }
                }
                // Straight to `set_custom_mode`, bypassing the advertised
                // list, which is how a client asks for a mode the compositor
                // may well refuse.
                "custom" => {
                    let (size, refresh) = match value.split_once('@') {
                        Some((size, refresh)) => {
                            (size, (refresh.parse::<f64>().unwrap_or(0.0) * 1000.0) as i32)
                        }
                        None => (value, 0),
                    };
                    if let Some((width, height)) = size.split_once('x') {
                        configured.set_custom_mode(
                            width.parse().unwrap_or(0),
                            height.parse().unwrap_or(0),
                            refresh,
                        );
                    }
                }
                "vrr" => configured.set_adaptive_sync(if value == "true" {
                    zwlr_output_head_v1::AdaptiveSyncState::Enabled
                } else {
                    zwlr_output_head_v1::AdaptiveSyncState::Disabled
                }),
                other => eprintln!("unknown setting {other}"),
            }
        }
    }

    if test_only {
        configuration.test();
    } else {
        configuration.apply();
    }

    while app.outcome.is_none() {
        queue.blocking_dispatch(app).expect("dispatch failed");
    }
    println!("{}", app.outcome.unwrap_or("no outcome"));
}

fn parse_transform(value: &str) -> Option<wl_output::Transform> {
    Some(match value {
        "normal" | "0" => wl_output::Transform::Normal,
        "90" => wl_output::Transform::_90,
        "180" => wl_output::Transform::_180,
        "270" => wl_output::Transform::_270,
        "flipped" => wl_output::Transform::Flipped,
        "flipped-90" => wl_output::Transform::Flipped90,
        "flipped-180" => wl_output::Transform::Flipped180,
        "flipped-270" => wl_output::Transform::Flipped270,
        _ => return None,
    })
}

fn find_mode(head: &Head, value: &str) -> Option<ZwlrOutputModeV1> {
    let (size, refresh) = match value.split_once('@') {
        Some((size, refresh)) => (size, refresh.parse::<f64>().ok()),
        None => (value, None),
    };
    let (width, height) = size.split_once('x')?;
    let (width, height) = (width.parse::<i32>().ok()?, height.parse::<i32>().ok()?);

    head.modes
        .iter()
        .filter(|(_, mode)| mode.width == width && mode.height == height)
        .min_by_key(|(_, mode)| match refresh {
            Some(refresh) => (mode.refresh as f64 - refresh * 1000.0).abs() as i64,
            None => -(mode.refresh as i64),
        })
        .map(|(handle, _)| handle.clone())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                state.order.push(head.clone());
                state.heads.insert(head, Head::default());
            }
            zwlr_output_manager_v1::Event::Done { serial } => state.serial = Some(serial),
            zwlr_output_manager_v1::Event::Finished => state.outcome = Some("manager finished"),
            _ => {}
        }
    }

    event_created_child!(App, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for App {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(head) = state.heads.get_mut(proxy) else {
            return;
        };
        match event {
            zwlr_output_head_v1::Event::Name { name } => head.name = name,
            zwlr_output_head_v1::Event::Description { description } => {
                head.description = description;
            }
            zwlr_output_head_v1::Event::PhysicalSize { width, height } => {
                head.physical = (width, height);
            }
            zwlr_output_head_v1::Event::Mode { mode } => head.modes.push((mode, Mode::default())),
            zwlr_output_head_v1::Event::Enabled { enabled } => head.enabled = enabled != 0,
            zwlr_output_head_v1::Event::CurrentMode { mode } => head.current_mode = Some(mode),
            zwlr_output_head_v1::Event::Position { x, y } => head.position = (x, y),
            zwlr_output_head_v1::Event::Transform { transform } => {
                head.transform = transform.into_result().ok();
            }
            zwlr_output_head_v1::Event::Scale { scale } => head.scale = scale,
            zwlr_output_head_v1::Event::Make { make } => head.make = Some(make),
            zwlr_output_head_v1::Event::Model { model } => head.model = Some(model),
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                head.serial = Some(serial_number);
            }
            zwlr_output_head_v1::Event::AdaptiveSync { state } => {
                head.adaptive_sync = Some(matches!(
                    state,
                    WEnum::Value(zwlr_output_head_v1::AdaptiveSyncState::Enabled)
                ));
            }
            zwlr_output_head_v1::Event::Finished => {
                state.heads.remove(proxy);
                state.order.retain(|existing| existing != proxy);
            }
            _ => {}
        }
    }

    event_created_child!(App, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for App {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some((_, mode)) = state
            .heads
            .values_mut()
            .flat_map(|head| head.modes.iter_mut())
            .find(|(handle, _)| handle == proxy)
        else {
            return;
        };

        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                mode.width = width;
                mode.height = height;
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => mode.refresh = refresh,
            zwlr_output_mode_v1::Event::Preferred => mode.preferred = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        state.outcome = Some(match event {
            zwlr_output_configuration_v1::Event::Succeeded => "succeeded",
            zwlr_output_configuration_v1::Event::Failed => "failed",
            zwlr_output_configuration_v1::Event::Cancelled => "cancelled",
            _ => return,
        });
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrOutputConfigurationHeadV1,
        _event: <ZwlrOutputConfigurationHeadV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

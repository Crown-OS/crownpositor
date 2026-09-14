//! Dumps what `wp_color_manager_v1` advertises, and reads an output's
//! description back.
//!
//! Nothing off the shelf exercises this protocol yet, so this is how the
//! capability burst and its version gating get checked against a real client.
//!
//! ```text
//! cargo run -p compositor --example color_info            # bind at 3
//! cargo run -p compositor --example color_info -- 1       # bind at 1
//! ```

use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_registry},
};
use wayland_protocols::wp::color_management::v1::client::{
    wp_color_management_output_v1::{self, WpColorManagementOutputV1},
    wp_color_manager_v1::{self, WpColorManagerV1},
    wp_image_description_info_v1::{self, WpImageDescriptionInfoV1},
    wp_image_description_v1::{self, WpImageDescriptionV1},
};

#[derive(Default)]
struct App {
    intents: Vec<String>,
    features: Vec<String>,
    transfer_functions: Vec<String>,
    primaries: Vec<String>,
    done: bool,
    output: Option<wl_output::WlOutput>,
    description_ready: Option<String>,
    info: Vec<String>,
    info_done: bool,
}

fn main() {
    let version: u32 = std::env::args()
        .nth(1)
        .and_then(|argument| argument.parse().ok())
        .unwrap_or(3);

    let connection = Connection::connect_to_env().expect("no Wayland compositor to talk to");
    let (globals, mut queue) =
        registry_queue_init::<App>(&connection).expect("the registry is unreadable");
    let qh = queue.handle();

    let manager: WpColorManagerV1 = globals
        .bind(&qh, version..=version, ())
        .expect("the compositor does not implement wp_color_management_v1 at that version");

    let mut app = App::default();
    app.output = globals.bind(&qh, 1..=4, ()).ok();

    while !app.done {
        queue.blocking_dispatch(&mut app).expect("dispatch failed");
    }

    println!("bound at version {version}");
    println!("  intents:    {}", app.intents.join(", "));
    println!("  features:   {}", app.features.join(", "));
    println!("  tf named:   {}", app.transfer_functions.join(", "));
    println!("  primaries:  {}", app.primaries.join(", "));

    let Some(output) = app.output.clone() else {
        println!("\n(no wl_output to describe)");
        return;
    };

    let color_output = manager.get_output(&output, &qh, ());
    let description = color_output.get_image_description(&qh, ());
    while app.description_ready.is_none() {
        queue.blocking_dispatch(&mut app).expect("dispatch failed");
    }
    println!("\noutput description: {}", app.description_ready.clone().unwrap_or_default());

    description.get_information(&qh, ());
    while !app.info_done {
        queue.blocking_dispatch(&mut app).expect("dispatch failed");
    }
    for line in &app.info {
        println!("  {line}");
    }
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

impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &wl_output::WlOutput,
        _event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpColorManagerV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &WpColorManagerV1,
        event: wp_color_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wp_color_manager_v1::Event::SupportedIntent { render_intent } => {
                state.intents.push(describe(render_intent));
            }
            wp_color_manager_v1::Event::SupportedFeature { feature } => {
                state.features.push(describe(feature));
            }
            wp_color_manager_v1::Event::SupportedTfNamed { tf } => {
                state.transfer_functions.push(describe(tf));
            }
            wp_color_manager_v1::Event::SupportedPrimariesNamed { primaries } => {
                state.primaries.push(describe(primaries));
            }
            wp_color_manager_v1::Event::Done => state.done = true,
            _ => {}
        }
    }
}

fn describe<T: std::fmt::Debug>(value: WEnum<T>) -> String {
    match value {
        WEnum::Value(value) => format!("{value:?}"),
        WEnum::Unknown(raw) => format!("unknown({raw})"),
    }
}

impl Dispatch<WpColorManagementOutputV1, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &WpColorManagementOutputV1,
        _event: wp_color_management_output_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpImageDescriptionV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &WpImageDescriptionV1,
        event: wp_image_description_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wp_image_description_v1::Event::Ready { identity } => {
                state.description_ready = Some(format!("ready, identity {identity}"));
            }
            wp_image_description_v1::Event::Ready2 { identity_hi, identity_lo } => {
                let identity = (u64::from(identity_hi) << 32) | u64::from(identity_lo);
                state.description_ready = Some(format!("ready2, identity {identity}"));
            }
            wp_image_description_v1::Event::Failed { cause, msg } => {
                state.description_ready = Some(format!("failed: {} ({msg})", describe(cause)));
                state.info_done = true;
            }
            _ => {}
        }
    }

    event_created_child!(App, WpImageDescriptionV1, [
        wp_image_description_v1::REQ_GET_INFORMATION_OPCODE => (WpImageDescriptionInfoV1, ()),
    ]);
}

impl Dispatch<WpImageDescriptionInfoV1, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &WpImageDescriptionInfoV1,
        event: wp_image_description_info_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wp_image_description_info_v1::Event::Primaries {
                r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y,
            } => state.info.push(format!(
                "primaries R({r_x},{r_y}) G({g_x},{g_y}) B({b_x},{b_y}) W({w_x},{w_y})"
            )),
            wp_image_description_info_v1::Event::PrimariesNamed { primaries } => {
                state.info.push(format!("primaries named {}", describe(primaries)));
            }
            wp_image_description_info_v1::Event::TfNamed { tf } => {
                state.info.push(format!("tf named {}", describe(tf)));
            }
            wp_image_description_info_v1::Event::TfPower { eexp } => {
                state.info.push(format!("tf power {}", f64::from(eexp) / 10_000.0));
            }
            wp_image_description_info_v1::Event::Luminances {
                min_lum, max_lum, reference_lum,
            } => state.info.push(format!(
                "luminances min {} max {max_lum} reference {reference_lum}",
                f64::from(min_lum) / 10_000.0
            )),
            wp_image_description_info_v1::Event::TargetPrimaries {
                r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y,
            } => state.info.push(format!(
                "target primaries R({r_x},{r_y}) G({g_x},{g_y}) B({b_x},{b_y}) W({w_x},{w_y})"
            )),
            wp_image_description_info_v1::Event::TargetLuminance { min_lum, max_lum } => {
                state.info.push(format!(
                    "target luminance min {} max {max_lum}",
                    f64::from(min_lum) / 10_000.0
                ));
            }
            wp_image_description_info_v1::Event::Done => state.info_done = true,
            _ => {}
        }
    }
}

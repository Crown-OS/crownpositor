//! The events that describe one window to one client.

use crownos_protocols::context::v1::server::crownos_context_window_v1::CrownosContextWindowV1;
use wayland_server::{DisplayHandle, Resource};

use super::{ContextDispatch, ContextRecord, Placement, WindowKey, bound::Bound};

/// Creates the window object for `record` and describes it. The title is
/// left out unless the client has the text scope.
pub(super) fn announce<D: ContextDispatch>(
    display: &DisplayHandle,
    bound: &mut Bound,
    record: &ContextRecord,
    with_text: bool,
) {
    let Some(client) = bound.manager.client() else {
        return;
    };
    let Ok(window) = client.create_resource::<CrownosContextWindowV1, WindowKey, D>(
        display,
        bound.manager.version(),
        record.key,
    ) else {
        return;
    };
    bound.manager.window(&window, record.identifier.clone());
    window.app_id(record.app_id.clone());
    if with_text {
        window.title(record.title.clone());
    }
    window.process(record.pid);
    window.sandbox(
        record.sandbox.engine.clone(),
        record.sandbox.app_id.clone(),
        record.sandbox.instance_id.clone(),
    );
    send_placement(&window, &record.placement);
    window.state(record.flags);
    window.content_type(record.content_type);
    window.sensitivity(record.sensitivity);
    window.done();
    bound.windows.insert(record.key, window);
}

pub(super) fn send_changes(
    window: &CrownosContextWindowV1,
    known: &ContextRecord,
    record: &ContextRecord,
    with_text: bool,
) {
    if known.app_id != record.app_id {
        window.app_id(record.app_id.clone());
    }
    if with_text && known.title != record.title {
        window.title(record.title.clone());
    }
    if known.pid != record.pid {
        window.process(record.pid);
    }
    if known.sandbox != record.sandbox {
        window.sandbox(
            record.sandbox.engine.clone(),
            record.sandbox.app_id.clone(),
            record.sandbox.instance_id.clone(),
        );
    }
    if known.placement != record.placement {
        send_placement(window, &record.placement);
    }
    if known.flags != record.flags {
        window.state(record.flags);
    }
    if known.content_type != record.content_type {
        window.content_type(record.content_type);
    }
    if known.sensitivity != record.sensitivity {
        window.sensitivity(record.sensitivity);
    }
}

fn send_placement(window: &CrownosContextWindowV1, placement: &Placement) {
    window.placement(
        placement.x,
        placement.y,
        placement.width,
        placement.height,
        placement.output.clone(),
        placement.workspace.clone(),
    );
}

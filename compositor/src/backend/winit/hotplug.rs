//! Switching nested outputs off and on, and unplugging and replugging them.
//!
//! Disabling keeps the viewport in the head list as a disabled head, the way
//! a monitor switched off through output management stays listed.
//! Disconnecting removes it from the list, the way a cable pulled does.

use crate::{backend::winit::viewport::Connection, state::State};

/// Turns a nested output on or off by its name. `Ok(false)` means there is
/// no such head.
pub fn set_head_enabled(state: &mut State, name: &str, enabled: bool) -> anyhow::Result<bool> {
    let Some(winit) = state.backend.winit() else {
        anyhow::bail!("this backend has no nested outputs");
    };
    let Some((index, connection)) = winit
        .viewports
        .iter()
        .enumerate()
        .find(|(_, viewport)| viewport.name == name)
        .map(|(index, viewport)| (index, viewport.connection))
    else {
        return Ok(false);
    };
    match (connection, enabled) {
        (Connection::Disconnected, _) => return Ok(false),
        (Connection::Connected, false) => remove_viewport(state, index, Connection::Disabled),
        (Connection::Disabled, true) => connect_viewport(state, index),
        _ => {}
    }
    Ok(true)
}

/// Unplugs the nested output `number` (counted from 1), or plugs it back in.
pub fn toggle_connection(state: &mut State, number: u8) {
    let Some(winit) = state.backend.winit() else {
        tracing::warn!(number, "nested-disconnect works only in a nested session");
        return;
    };
    let Some((index, connection)) = usize::from(number)
        .checked_sub(1)
        .and_then(|index| Some((index, winit.viewports.get(index)?.connection)))
    else {
        tracing::warn!(number, "there is no such nested output");
        return;
    };
    match connection {
        Connection::Connected => remove_viewport(state, index, Connection::Disconnected),
        Connection::Disconnected => connect_viewport(state, index),
        Connection::Disabled => {
            if let Some(viewport) = winit.viewports.get_mut(index) {
                viewport.connection = Connection::Disconnected;
            }
            state.refresh_output_heads();
            tracing::info!(number, "disabled nested output unplugged");
        }
    }
}

fn remove_viewport(state: &mut State, index: usize, connection: Connection) {
    let Some(output) = state
        .backend
        .winit()
        .and_then(|winit| winit.viewports.get_mut(index))
        .map(|viewport| {
            viewport.connection = connection;
            viewport.target = None;
            viewport.output.clone()
        })
    else {
        return;
    };
    state.capture.output_removed(&output);
    state
        .shell
        .remove_output(&state.common.display_handle, &output);
    state.refresh_output_heads();
    state.queue_redraw();
    tracing::info!(output = output.name(), ?connection, "nested output removed");
}

/// Adds the viewport's output again, from the same descriptor, so the shell
/// sees the same connector come back.
fn connect_viewport(state: &mut State, index: usize) {
    let Some(descriptor) = state.backend.winit().and_then(|winit| {
        let transform = winit.output_transform();
        winit
            .viewports
            .get(index)
            .map(|viewport| viewport.descriptor(transform))
    }) else {
        return;
    };
    let output = state.shell.add_output(
        &state.common.display_handle,
        &state.config.current,
        descriptor,
    );
    let name = output.name();
    if let Some(viewport) = state
        .backend
        .winit()
        .and_then(|winit| winit.viewports.get_mut(index))
    {
        viewport.reconnect(output);
    }
    state.refresh_output_heads();
    state.queue_redraw();
    tracing::info!(output = name, "nested output added");
}

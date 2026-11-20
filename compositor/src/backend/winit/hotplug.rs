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

/// Lights the viewports the config and the saved arrangement want on, as a
/// cold plug does on hardware.
pub fn bring_up_all(state: &mut State) {
    state.sync_offline_heads();
    let names: Vec<String> = state
        .backend
        .winit()
        .map(|winit| {
            winit
                .viewports
                .iter()
                .map(|viewport| viewport.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let connectors: Vec<&str> = names.iter().map(String::as_str).collect();
    let lit: Vec<String> = state
        .connectors_to_light(&connectors)
        .into_iter()
        .map(str::to_owned)
        .collect();
    for (index, name) in names.iter().enumerate() {
        if lit.contains(name) {
            connect_viewport(state, index);
        } else {
            tracing::info!(
                output = name,
                "nested output stays off; the config disables it"
            );
        }
    }
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
        Connection::Connected => {
            remove_viewport(state, index, Connection::Disconnected);
            state.reconcile_output_layout();
        }
        Connection::Disconnected => {
            plug_viewport(state, index);
            state.reconcile_output_layout();
        }
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
    let State { backend, shell, .. } = &mut *state;
    let Some(output) = backend
        .winit()
        .and_then(|winit| winit.viewports.get_mut(index))
        .map(|viewport| {
            viewport.connection = connection;
            viewport.target = None;
            viewport.last_position = shell
                .monitor(&viewport.output)
                .map(|monitor| monitor.config().position);
            viewport.output.clone()
        })
    else {
        return;
    };
    state.output_removed(&output);
    tracing::info!(output = output.name(), ?connection, "nested output removed");
}

/// Plugs a viewport back in: lit, or listed as switched off when the config
/// or the saved arrangement has it off, as a monitor plugged in does.
fn plug_viewport(state: &mut State, index: usize) {
    let Some(name) = state.backend.winit().and_then(|winit| {
        let viewport = winit.viewports.get_mut(index)?;
        viewport.connection = Connection::Disabled;
        Some(viewport.name.clone())
    }) else {
        return;
    };
    state.sync_offline_heads();
    if state.should_enable_head(&name) {
        connect_viewport(state, index);
    } else {
        state.refresh_output_heads();
        tracing::info!(
            output = name,
            "nested output plugged in, off as the config asks"
        );
    }
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
        viewport.reconnect(output.clone());
    }
    state.output_added(&output);
    tracing::info!(output = name, "nested output added");
}

//! Outputs with no display: `crownos_virtual_output_v1`.
//!
//! A virtual output enters the shell through the same door as a monitor —
//! [`Shell::add_output`] — so it gets workspaces, a layer map, a `wl_output`
//! global and an output-management head like any other. What it does not get
//! is a scanout loop: nothing draws it except the capture sessions pointed at
//! it, and its frame clock ticks only when its client calls `request_frame`.
//!
//! [`Shell::add_output`]: crate::shell::Shell::add_output

use std::time::Duration;

use smithay::{
    desktop::{layer_map_for_output, utils::OutputPresentationFeedback},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
    utils::Transform,
    wayland::presentation::Refresh,
};

use protocols::crownos_virtual_output::{CloseReason, VirtualMode, VirtualOutput, full_name};

use crate::{
    backend::present::{self, ReleasedClients},
    shell::monitor::OutputDescriptor,
    state::State,
};

struct VirtualHead {
    protocol: VirtualOutput,
    output: Output,
    frame_requested: bool,
}

#[derive(Default)]
pub struct VirtualOutputs {
    heads: Vec<VirtualHead>,
}

impl VirtualOutputs {
    pub fn contains(&self, output: &Output) -> bool {
        self.heads.iter().any(|head| head.output == *output)
    }

    fn position(&self, protocol: &VirtualOutput) -> Option<usize> {
        self.heads
            .iter()
            .position(|head| head.protocol.id() == protocol.id())
    }
}

fn smithay_mode(mode: VirtualMode) -> Mode {
    Mode {
        size: mode.size,
        refresh: mode.refresh(),
    }
}

fn refresh_interval(mode: VirtualMode) -> Duration {
    Duration::from_nanos(1_000_000_000_000 / u64::from(mode.refresh_mhz.max(1)))
}

impl State {
    pub fn create_virtual_output(
        &mut self,
        protocol: VirtualOutput,
        name: &str,
        mode: VirtualMode,
    ) {
        let name = full_name(name);
        if self.shell.monitor_by_name(&name).is_some() {
            protocol.closed(CloseReason::NameTaken);
            return;
        }

        let current = smithay_mode(mode);
        let output = self.shell.add_output(
            &self.common.display_handle,
            &self.config.current,
            OutputDescriptor {
                name: name.clone(),
                physical: PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "CrownOS".into(),
                    model: "Virtual Output".into(),
                    serial_number: name.clone(),
                },
                modes: vec![current],
                preferred: Some(current),
                current,
                native_transform: Transform::Normal,
                refresh_interval: Some(refresh_interval(mode)),
                serial: None,
                edid: None,
            },
        );
        self.apply_virtual_scale(&output, mode);

        self.virtual_outputs.heads.push(VirtualHead {
            protocol: protocol.clone(),
            output,
            frame_requested: true,
        });
        self.refresh_output_heads();
        protocol.created(&name);
        tracing::info!(output = name, ?mode, "virtual output created");
    }

    pub fn set_virtual_output_mode(&mut self, protocol: &VirtualOutput, mode: VirtualMode) {
        let Some(output) = self
            .virtual_outputs
            .position(protocol)
            .map(|index| self.virtual_outputs.heads[index].output.clone())
        else {
            return;
        };
        let current = smithay_mode(mode);
        if let Some(previous) = output
            .current_mode()
            .filter(|previous| *previous != current)
        {
            output.delete_mode(previous);
        }
        output.add_mode(current);
        output.set_preferred(current);
        if let Some(monitor) = self.shell.monitor_mut(&output) {
            let config = monitor.config_mut();
            config.modes = vec![current];
            config.preferred_mode = Some(current);
            monitor.set_mode(current);
        }
        self.apply_virtual_scale(&output, mode);
        self.capture.mark_output_damaged(&output);
        self.refresh_output_heads();
    }

    pub fn request_virtual_output_frame(&mut self, protocol: &VirtualOutput) {
        if let Some(index) = self.virtual_outputs.position(protocol) {
            self.virtual_outputs.heads[index].frame_requested = true;
        }
    }

    pub fn remove_virtual_output(&mut self, protocol: &VirtualOutput) {
        let Some(index) = self.virtual_outputs.position(protocol) else {
            return;
        };
        let head = self.virtual_outputs.heads.swap_remove(index);
        self.capture.output_removed(&head.output);
        self.shell
            .remove_output(&self.common.display_handle, &head.output);
        self.refresh_output_heads();
        self.queue_redraw();
        tracing::info!(output = head.output.name(), "virtual output removed");
    }

    fn apply_virtual_scale(&mut self, output: &Output, mode: VirtualMode) {
        if let Some(monitor) = self.shell.monitor_mut(output) {
            monitor.set_scale(mode.scale());
        }
        self.shell.arrange_outputs();
        self.shell.refresh_usable(output);
        self.shell.advertise_output_scale(output);
    }
}

/// Advances every virtual output a frame was asked for: steps the
/// animations and marks its capture sessions, which render next. Returns the
/// outputs whose surfaces are owed frame callbacks once they have.
pub fn begin_frames(state: &mut State) -> Vec<Output> {
    let outputs: Vec<Output> = state
        .virtual_outputs
        .heads
        .iter_mut()
        .filter_map(|head| std::mem::take(&mut head.frame_requested).then(|| head.output.clone()))
        .collect();
    if outputs.is_empty() {
        return outputs;
    }

    let now: Duration = state.wayland.clock.now().into();
    let dt = state.clock.tick_to(now);
    state.shell.advance_animations(dt);
    if !state.shell.is_animating() {
        state.shell.settle_animations();
    }
    for output in &outputs {
        state.capture.mark_output_damaged(output);
    }
    outputs
}

/// Frame callbacks and presentation feedback for the surfaces on `outputs`,
/// after their frame has been captured.
pub fn end_frames(state: &mut State, outputs: &[Output]) {
    let now = state.common.start_time.elapsed();
    let presented: Duration = state.wayland.clock.now().into();
    let throttle = Some(Duration::ZERO);

    for output in outputs {
        let Some(monitor) = state.shell.monitor(output) else {
            continue;
        };
        let refresh = output
            .current_mode()
            .filter(|mode| mode.refresh > 0)
            .map(|mode| {
                Refresh::Fixed(Duration::from_nanos(
                    1_000_000_000_000 / mode.refresh as u64,
                ))
            })
            .unwrap_or(Refresh::Unknown);
        let mut feedback = OutputPresentationFeedback::new(output);
        let flags = |_: &_, _: &_| wp_presentation_feedback::Kind::empty();

        for tile in state.shell.visible_windows(monitor) {
            let window = tile.window();
            window.take_presentation_feedback(&mut feedback, |_, _| Some(output.clone()), flags);
            window.send_frame(output, now, throttle, |_, _| Some(output.clone()));
        }
        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.take_presentation_feedback(&mut feedback, |_, _| Some(output.clone()), flags);
            layer.send_frame(output, now, throttle, |_, _| Some(output.clone()));
        }
        state.input.cursor.send_frame(output, now, throttle);

        feedback.presented::<_, smithay::utils::Monotonic>(
            presented,
            refresh,
            0,
            wp_presentation_feedback::Kind::empty(),
        );
    }

    // Nothing tracks which output a virtual one's surfaces are shown on, so
    // their FIFO barriers are released as "shown nowhere else".
    let mut released = ReleasedClients::default();
    for output in outputs {
        present::signal_fifo_barriers(&state.shell, output, &state.input.cursor, &mut released);
    }
    state.clear_blockers(released);
}

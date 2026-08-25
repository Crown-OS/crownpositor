//! The single action dispatch point.
//!
//! Every arm mutates the model and nothing else; geometry, focus and the
//! invariant check happen once at the end.

use std::{os::unix::process::CommandExt, process::Stdio};

use smithay::utils::Serial;

use config::{Config, Update};

use crate::{
    animations::spring::SpringProfile,
    handlers::seat::KeyboardFocusTarget,
    input::shortcuts::{Action, Bindings},
    layout::{Gaps, LayoutOp},
    shell::tile::WindowState,
    state::State,
};

impl State {
    pub fn handle_action(&mut self, action: Action, serial: Serial) {
        match action {
            Action::None => return,

            Action::Quit => {
                tracing::info!("quit requested");
                self.common.event_loop_signal.stop();
                return;
            }

            Action::ReloadConfig => {
                self.apply_config(Config::load());
            }

            Action::Spawn(argv) => {
                self.spawn(&argv);
            }

            Action::SwitchVt(vt) => {
                // Nothing local changes: the session notifier will pause us and
                // force full redraws when the seat comes back.
                self.backend.switch_vt(vt);
                return;
            }

            Action::CloseWindow => {
                if let Some(window) = self.shell.activated.clone()
                    && let Some(toplevel) = window.toplevel()
                {
                    toplevel.send_close();
                }
            }

            Action::Focus(dir) => {
                self.shell.focus_direction(dir.into());
            }
            Action::MoveWindow(dir) => {
                self.shell.move_focused(dir.into());
            }
            Action::FocusOutput(dir) => {
                self.shell.focus_output_direction(dir.into());
            }
            Action::MoveWindowToOutput(dir) => {
                self.shell.move_focused_to_output(dir.into());
            }

            Action::Workspace(target) => {
                self.shell.switch_workspace(target);
            }
            Action::MoveWindowToWorkspace { target, follow } => {
                self.shell.move_focused_to_workspace(target, follow);
            }

            Action::ToggleFloating => {
                self.shell.toggle_floating();
            }
            Action::ToggleFullscreen => {
                self.shell.toggle_window_state(WindowState::Fullscreen);
            }
            Action::ToggleMaximize => {
                self.shell.toggle_window_state(WindowState::Maximized);
            }

            Action::ToggleLayoutMode => {
                self.shell.toggle_global_layout();
            }
            Action::CycleLayout => {
                self.shell.cycle_workspace_layout();
            }
            Action::SetLayout(selection) => {
                self.shell.set_workspace_layout(selection.into());
            }

            Action::ResizeSplit(fraction) => {
                self.shell.apply_layout_op(LayoutOp::Grow(fraction));
            }
            Action::PromoteDemote => {
                if let Some(id) = self.shell.focused_window_id() {
                    self.shell.apply_layout_op(LayoutOp::PromoteDemote(id));
                }
            }
            Action::CycleSize => {
                if let Some(id) = self.shell.focused_window_id() {
                    self.shell.apply_layout_op(LayoutOp::CyclePreset(id));
                }
            }
            Action::ResetSize => {
                if let Some(id) = self.shell.focused_window_id() {
                    self.shell.apply_layout_op(LayoutOp::ResetSize(id));
                }
            }

            // TODO: needs the exposé view in `shell/workspaces_view`.
            Action::MoveWorkspaceToOutput(_)
            | Action::OpenWorkspaceView
            | Action::CloseWorkspaceView => {
                tracing::warn!(?action, "action is not implemented yet");
            }
        }

        // One place recomputes geometry, one place moves keyboard focus.
        self.shell.refresh();
        self.update_keyboard_focus(serial);
        self.queue_redraw();
    }

    /// The only caller of `set_focus` outside a pointer click.
    pub fn update_keyboard_focus(&mut self, serial: Serial) {
        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return;
        };
        let target = self
            .shell
            .focused_window_id()
            .and_then(|id| self.shell.tile(id))
            .map(|tile| KeyboardFocusTarget::from(tile.window().clone()));

        if keyboard.current_focus() == target {
            return;
        }
        keyboard.set_focus(self, target, serial);
    }

    /// Window rules are deliberately not retro-applied: a window floated by hand
    /// must not be re-tiled because an unrelated rule was edited.
    pub fn apply_config(&mut self, new: Config) {
        self.input.bindings = Bindings::from_config(&new.keybinds);

        self.shell.set_global_layout(new.compositor.layout.into());
        self.shell.set_gaps(Gaps {
            inner: new.appearance.gaps_inner.into(),
            outer: new.appearance.gaps_outer.into(),
        });
        self.shell
            .set_workspace_animation(SpringProfile::from_config(new.appearance.animations));

        self.config.current = new;
        self.shell.apply_output_settings(&self.config.current);
        // The next refresh turns the dirty bits above into one relayout.
    }

    /// One watched key changed, so only the component that owns it re-runs.
    pub fn apply_update(&mut self, update: Update) {
        tracing::info!(?update, "config changed");
        let config = &mut self.config.current;

        match update {
            Update::Layout(layout) => {
                config.compositor.layout = layout;
                self.shell.set_global_layout(layout.into());
            }
            Update::FocusFollowsMouse(follows) => config.compositor.focus_follows_mouse = follows,
            Update::WindowRules(rules) => config.window_rules = rules,

            Update::Outputs(outputs) => {
                config.compositor.outputs = outputs;
                self.shell.apply_output_settings(&self.config.current);
            }
            Update::Scale(scale) => {
                config.display.scale = scale;
                self.shell.apply_output_settings(&self.config.current);
            }

            Update::Keybinds(keybinds) => {
                self.input.bindings = Bindings::from_config(&keybinds);
                config.keybinds = keybinds;
            }
            Update::Appearance(appearance) => {
                self.shell.set_gaps(Gaps {
                    inner: appearance.gaps_inner.into(),
                    outer: appearance.gaps_outer.into(),
                });
                self.shell
                    .set_workspace_animation(SpringProfile::from_config(appearance.animations));
                config.appearance = appearance;
            }
        }

        self.queue_redraw();
    }

    pub fn run_startup(&self) {
        for argv in config::startup::commands(&self.config.current.compositor.startup) {
            tracing::info!(command = %argv.join(" "), "startup");
            self.spawn(&argv);
        }
    }

    /// `setsid` reparents the child to init, so the compositor never has to reap.
    fn spawn(&self, argv: &[String]) {
        let Some((program, args)) = argv.split_first() else {
            return;
        };

        let mut command = std::process::Command::new(program);
        command
            .args(args)
            .env("WAYLAND_DISPLAY", &self.common.socket_name)
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Safety: `setsid` is async-signal-safe and the only call between fork
        // and exec.
        unsafe {
            command.pre_exec(|| {
                let _ = smithay::reexports::rustix::process::setsid();
                Ok(())
            });
        }

        match command.spawn() {
            Ok(child) => tracing::debug!(program, pid = child.id(), "spawned"),
            Err(err) => tracing::warn!(%err, program, "failed to spawn"),
        }
    }
}

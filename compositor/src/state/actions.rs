//! The single action dispatch point.
//!
//! Every arm mutates the model and nothing else; geometry, focus and the
//! invariant check happen once at the end.

use std::{os::unix::process::CommandExt, process::Stdio};

use smithay::{backend::input::InputTime, input::pointer::MotionEvent, utils::SERIAL_COUNTER};

use config::{Config, Update};
use protocols::{
    background_effect::Capability as BackgroundEffectCapability,
    crownos_background_effects::Capability as CrownosEffectCapability,
};

use spacecontrol::animations::spring::SpringProfile;

use crate::{
    input::shortcuts::{Action, Bindings},
    layout::{Gaps, LayoutOp},
    shell::tile::WindowState,
    state::State,
};

impl State {
    pub fn handle_action(&mut self, action: Action) {
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

            Action::ToggleWorkspaceMode => {
                self.shell.toggle_workspace_mode();
            }
            Action::SetWorkspaceMode(mode) => {
                self.shell.set_workspace_mode(mode);
            }
            Action::SnapWindow(zone) => {
                self.shell.snap_focused(zone);
            }
            Action::CloseMenu => {
                self.dismiss_menu();
            }

            Action::ResizeSplit(fraction) => {
                self.shell.apply_layout_op(LayoutOp::Grow(fraction));
            }
            Action::PromoteDemote => {
                if let Some(id) = self.shell.focused_window_id() {
                    self.shell.apply_layout_op(LayoutOp::PromoteDemote(id));
                }
            }
            Action::ResetSize => {
                self.shell.apply_layout_op(LayoutOp::ResetSize);
            }

            Action::OpenWorkspaceView => {
                if let Some(monitor) = self.shell.focused_monitor_mut() {
                    monitor.with_spacecontrol(|space, monitor| space.open(monitor));
                }
            }
            Action::CloseWorkspaceView => {
                if let Some(monitor) = self.shell.focused_monitor_mut() {
                    monitor.spacecontrol_mut().close();
                }
            }
            Action::ToggleWorkspaceView => {
                if let Some(monitor) = self.shell.focused_monitor_mut() {
                    monitor.with_spacecontrol(|space, monitor| space.toggle(monitor));
                }
            }

            Action::MoveWorkspaceToOutput(_) => {
                tracing::warn!(?action, "action is not implemented yet");
            }
        }

        // One place recomputes geometry, one place moves keyboard focus.
        self.shell.refresh();
        self.update_keyboard_focus();
        self.queue_redraw();
    }

    /// Brings the seat's keyboard focus in line with the model. The only caller
    /// of `set_focus`.
    ///
    /// Cheap to call on any change and safe to call twice: it compares against
    /// the focus the seat already has and sends nothing when it matches, so the
    /// event loop can run it once per dispatch rather than every mutation
    /// having to remember to.
    pub fn update_keyboard_focus(&mut self) {
        let Some(keyboard) = self.wayland.seat.get_keyboard() else {
            return;
        };
        let target = self.shell.keyboard_focus();

        if keyboard.current_focus() == target {
            return;
        }

        tracing::debug!(?target, "keyboard focus moved");
        keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
    }

    /// The same reconcile for the pointer, for the changes no pointer event
    /// comes with: a window mapping under a still cursor, a relayout, a
    /// workspace switch. The surface under the cursor would otherwise never
    /// hear `enter`, and everything addressed to the focus — scroll, and
    /// gestures like pinch, which carry no motion of their own — would keep
    /// going to whatever was under the cursor before.
    ///
    /// Compares the target first, so a dispatch that moved nothing sends
    /// nothing. A grab owns the pointer outright and an open menu holds it for
    /// as long as it is open, so neither is disturbed.
    pub fn update_pointer_focus(&mut self) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let location = self.input.pointer_location;
        if pointer.is_grabbed() || self.shell.menus.contains(location) {
            return;
        }

        // The overview owns the pointer outright, so every client is told it
        // left and hears nothing more until it closes. Handing that leave over
        // is also what takes the cursor back: a client that had hidden it has
        // no say over the pointer any more.
        let under = match self.overview_owns_input() {
            true => {
                if self.input.cursor.reclaim() {
                    self.queue_pointer_redraw();
                }
                None
            }
            false => self.shell.pointer_focus_under(location),
        };
        if pointer.current_focus().as_ref() == under.as_ref().map(|(target, _)| target) {
            return;
        }

        self.track_frame_hover(
            under
                .as_ref()
                .is_some_and(|(target, _)| target.is_decoration()),
        );
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time: InputTime::now(),
            },
        );
        pointer.frame(self);
    }

    /// Window rules are deliberately not retro-applied: a window floated by hand
    /// must not be re-tiled because an unrelated rule was edited.
    pub fn apply_config(&mut self, new: Config) {
        self.input.bindings = Bindings::with_custom(&new.keybinds.custom_keybinds);

        self.shell.set_default_mode(new.compositor.layout);
        self.shell.set_gaps(Gaps {
            inner: new.appearance.gaps_inner.into(),
            outer: new.appearance.gaps_outer.into(),
        });
        self.shell
            .set_titlebar_height(new.appearance.titlebar_height.into());
        self.shell
            .set_workspace_animation(SpringProfile::from_config(new.appearance.animations));

        self.config.current = new;
        self.shell.apply_output_settings(&self.config.current);
        self.refresh_output_heads();
        self.sync_background_effect_capabilities();
        // The next refresh turns the dirty bits above into one relayout.
    }

    /// Keeps the `ext-background-effect-v1` capabilities in step with what the
    /// compositor will actually draw.
    ///
    /// The protocol is explicit that a withdrawn capability stops being applied
    /// "even if it was set before", which is precisely what turning blur off in
    /// the config does — so clients are told, and a panel that was relying on
    /// glass can go back to painting its own background.
    pub fn sync_background_effect_capabilities(&mut self) {
        let blur = self.config.current.appearance.blur;

        self.wayland
            .background_effect_state
            .set_capabilities(if blur {
                BackgroundEffectCapability::Blur
            } else {
                BackgroundEffectCapability::empty()
            });

        // The corner radius, the rim and the shadow are geometry the renderer
        // draws whether or not the blur pipeline exists, so only the blur
        // itself follows the setting. The rim refracts the blurred backdrop, so
        // it goes with it.
        let mut crownos = CrownosEffectCapability::CornerRadius | CrownosEffectCapability::Shadow;
        crownos.set(
            CrownosEffectCapability::Blur | CrownosEffectCapability::Border,
            blur,
        );
        self.wayland
            .crownos_background_effects_state
            .set_capabilities(crownos);
    }

    /// One watched key changed, so only the component that owns it re-runs.
    pub fn apply_update(&mut self, update: Update) {
        tracing::info!(?update, "config changed");
        let config = &mut self.config.current;

        match update {
            Update::DefaultMode(mode) => {
                config.compositor.layout = mode;
                self.shell.set_default_mode(mode);
            }
            Update::FocusFollowsMouse(follows) => config.compositor.focus_follows_mouse = follows,
            Update::WindowRules(rules) => config.window_rules = rules,

            Update::Outputs(outputs) => {
                config.compositor.outputs = outputs;
                self.shell.apply_output_settings(&self.config.current);
                self.refresh_output_heads();
            }
            Update::OutputLayouts(layouts) => {
                config.compositor.output_layouts = layouts;
                self.shell.apply_output_settings(&self.config.current);
                self.refresh_output_heads();
            }

            Update::CustomKeybinds(custom) => {
                self.input.bindings = Bindings::with_custom(&custom);
                config.keybinds.custom_keybinds = custom;
            }
            Update::Appearance(appearance) => {
                self.shell.set_gaps(Gaps {
                    inner: appearance.gaps_inner.into(),
                    outer: appearance.gaps_outer.into(),
                });
                self.shell
                    .set_titlebar_height(appearance.titlebar_height.into());
                self.shell
                    .set_workspace_animation(SpringProfile::from_config(appearance.animations));
                config.appearance = appearance;
                self.sync_background_effect_capabilities();
            }
            Update::Display(display) => {
                config.display = display;
                self.apply_display_gamma();
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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Set per child rather than process-wide, because an inherited
        // `DISPLAY` from a host X session has to be *removed* when we have no
        // X11 support of our own — otherwise every child quietly talks to
        // somebody else's X server.
        match self.xwayland.display_name() {
            Some(display) => command.env("DISPLAY", display),
            None => command.env_remove("DISPLAY"),
        };

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

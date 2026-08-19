//! Backends: the things that own a renderer, real outputs and real input.
//!
//! # Adding one
//!
//! A backend is a module with an `init(&mut State) -> anyhow::Result<()>` that
//! does four things:
//!
//! 1. **Registers its outputs** by calling [`Shell::add_output`] with an
//!    [`OutputDescriptor`]. It must not call `Output::new` or `create_global`
//!    itself — the shell does both, which is what makes registration impossible
//!    to forget. Forgetting it is what silently dropped every layer surface
//!    before.
//! 2. **Inserts its event source** into `state.common.event_loop_handle`,
//!    forwarding input through [`State::process_input_event`] and calling its own
//!    render function on whatever "time to draw" signal it has.
//! 3. **Builds its element list** with [`rendering::output_elements`], passing a
//!    [`TileDecorator`] for whatever its renderer can do to a window
//!    ([`PassThrough`] if nothing). Nothing in `rendering` is renderer-specific.
//! 4. **Stores itself** in `state.backend` as a new [`BackendState`] variant, and
//!    adds an arm to that enum's two methods.
//!
//! Everything above the backend — layout, workspaces, focus, config — is already
//! renderer- and output-count-agnostic, so a backend should not need to touch it.
//!
//! [`Shell::add_output`]: crate::shell::Shell::add_output
//! [`OutputDescriptor`]: crate::shell::monitor::OutputDescriptor
//! [`State::process_input_event`]: crate::state::State::process_input_event
//! [`rendering::output_elements`]: crate::rendering::output_elements
//! [`TileDecorator`]: crate::rendering::decorate::TileDecorator
//! [`PassThrough`]: crate::rendering::decorate::PassThrough
//! [`BackendState`]: crate::state::BackendState

pub mod frame_clock;
pub mod kms;
pub mod render;
pub mod winit;

/// Which backend to start, chosen from the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preference {
    /// Nested inside an existing Wayland or X11 session.
    Winit,
    /// A bare TTY: DRM/KMS outputs, libinput input, libseat session.
    Kms,
}

impl Preference {
    /// A running session means we are nested; a bare TTY means DRM. The
    /// `CROWN_BACKEND` variable overrides the heuristic (`winit` / `kms`),
    /// because "run the KMS backend nested under a session that leaks
    /// `DISPLAY`" is a real debugging situation.
    pub fn detect() -> Self {
        if let Ok(value) = std::env::var("CROWN_BACKEND") {
            match value.to_ascii_lowercase().as_str() {
                "winit" => return Self::Winit,
                "kms" | "drm" | "udev" => return Self::Kms,
                other => {
                    tracing::warn!(requested = other, "unknown CROWN_BACKEND, autodetecting");
                }
            }
        }

        let nested = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var_os("DISPLAY").is_some();

        if nested { Self::Winit } else { Self::Kms }
    }
}

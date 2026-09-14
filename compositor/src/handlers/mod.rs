mod appmenu;
mod background_effect;
mod color_management;
mod compositor;
mod dmabuf;
mod drm_lease;
mod fractional_scale;
mod gamma_control;
mod idle_inhibit;
mod idle_notify;
mod kde_decoration;
mod keyboard_shortcuts_inhibit;
mod layer_shell;
mod output;
mod output_management;
mod output_power;
pub mod seat;
mod selection;
mod session_lock;
mod shm;
mod xdg_decoration;
mod xdg_shell;

use smithay::{delegate_presentation, delegate_viewporter};

use crate::state::State;

delegate_presentation!(State);
delegate_viewporter!(State);

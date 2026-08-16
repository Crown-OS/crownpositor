mod compositor;
mod dmabuf;
mod fractional_scale;
mod idle_inhibit;
mod idle_notify;
mod kde_decoration;
mod keyboard_shortcuts_inhibit;
mod output;
mod seat;
mod selection;
mod session_lock;
mod shm;
mod xdg_decoration;
mod xdg_shell;

use smithay::{delegate_presentation, delegate_viewporter};

use crate::state::State;

delegate_presentation!(State);
delegate_viewporter!(State);

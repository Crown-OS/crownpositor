//! The contract of the crownpositor control socket, shared by the compositor
//! and the apps that drive it.

crownos_ipc::protocol! {
    pub mod crownpositor: "crownpositor" {
        /// Runs one compositor action, spelled as in a keybinding. The
        /// compositor refuses `spawn` and `switch-vt`, and every action
        /// while the session is locked.
        fn action(spelling: String);
    }
}

#[cfg(test)]
mod tests;

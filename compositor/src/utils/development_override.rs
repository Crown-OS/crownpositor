//! A development build of a trusted program, for nested tests.

use std::path::PathBuf;

const BACKEND_VARIABLE: &str = "CROWN_BACKEND";
const NESTED_BACKEND: &str = "winit";

/// The path in `variable` of the compositor's own environment, honoured only
/// in a debug build or a nested session.
pub fn development_path(variable: &str) -> Option<PathBuf> {
    let nested = std::env::var(BACKEND_VARIABLE).is_ok_and(|backend| backend == NESTED_BACKEND);
    (cfg!(debug_assertions) || nested)
        .then(|| std::env::var_os(variable).map(PathBuf::from))
        .flatten()
}

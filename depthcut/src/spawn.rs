use std::{
    env,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use crate::DepthCutError;

pub const HELPER_NAME: &str = "crown-depthcut";

/// Starts the helper cutting `image`'s subject into [`mask_path`](crate::mask_path).
/// The caller waits on the child off its UI or render thread; a zero exit
/// means the mask is in place.
pub fn spawn_cut(image: &Path) -> Result<Child, DepthCutError> {
    Command::new(helper())
        .arg(image)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map_err(DepthCutError::Spawn)
}

/// The helper next to the running binary, as in a build tree, or else the
/// one on `PATH`.
fn helper() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(HELPER_NAME)))
        .filter(|beside| beside.is_file())
        .unwrap_or_else(|| PathBuf::from(HELPER_NAME))
}

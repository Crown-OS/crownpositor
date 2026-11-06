use std::{
    env,
    fs::File,
    io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

const MODEL_NAME: &str = "depth-anything-v2-small-int8.onnx";
const MODELS_SUBDIR: &str = "crownos/models";
const SYSTEM_DATA_DIR: &str = "/usr/share";
/// onnx-community/depth-anything-v2-small `onnx/model_int8.onnx`.
const MODEL_SHA256: &str = "01aa7a23de3f4a0ee1a2bb9997e6918104c85a9f95dea46d27b9b3fb0c6b9001";

/// The installed model: the user's copy under `$XDG_DATA_HOME` first, then
/// the system's, checked against the digest it was packaged with.
pub fn model_file() -> Result<PathBuf> {
    let file = candidates().find(|file| file.is_file()).with_context(|| {
        format!(
            "no depth model: install {MODEL_NAME} into $XDG_DATA_HOME/{MODELS_SUBDIR} \
             or {SYSTEM_DATA_DIR}/{MODELS_SUBDIR}"
        )
    })?;
    verify(&file).with_context(|| format!("cannot use {}", file.display()))?;
    Ok(file)
}

fn candidates() -> impl Iterator<Item = PathBuf> {
    let data_home = env::var_os("XDG_DATA_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    data_home
        .into_iter()
        .chain([PathBuf::from(SYSTEM_DATA_DIR)])
        .map(|dir| dir.join(MODELS_SUBDIR).join(MODEL_NAME))
}

fn verify(file: &Path) -> Result<()> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(file)?, &mut hasher)?;
    let digest = format!("{:x}", hasher.finalize());
    ensure!(
        digest == MODEL_SHA256,
        "its sha256 is {digest}, not {MODEL_SHA256}"
    );
    Ok(())
}

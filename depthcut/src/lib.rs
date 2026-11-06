//! The depth effect's subject masks: where each wallpaper's mask is cached,
//! and the helper process that cuts one.
//!
//! The segmentation model never lives in a host. A host asks
//! [`cached_mask`], and on a miss starts [`spawn_cut`] off its UI or render
//! thread; the helper loads the model, writes the mask and exits, which frees
//! every byte the model took.

mod cache;
mod spawn;

pub use cache::{cached_mask, mask_path};
pub use spawn::{HELPER_NAME, spawn_cut};

#[derive(Debug, thiserror::Error)]
pub enum DepthCutError {
    #[error("no cache directory: neither XDG_CACHE_HOME nor HOME is set")]
    NoCacheDir,
    #[error("cannot read {path}: {source}")]
    Unreadable {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("cannot start {HELPER_NAME}: {0}")]
    Spawn(std::io::Error),
}

use std::{
    env,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use crate::DepthCutError;

const CACHE_SUBDIR: &str = "crownos/depth";
const MASK_EXTENSION: &str = "png";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Where the mask for `image` lives, cut or not: keyed by the picture's
/// canonical path, size and modification time, so an edited or replaced
/// picture gets a fresh cut.
pub fn mask_path(image: &Path) -> Result<PathBuf, DepthCutError> {
    let unreadable = |source| DepthCutError::Unreadable {
        path: image.to_owned(),
        source,
    };
    let canonical = image.canonicalize().map_err(unreadable)?;
    let metadata = canonical.metadata().map_err(unreadable)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    let key = fnv1a(
        canonical
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .chain(&metadata.len().to_le_bytes())
            .chain(&modified.to_le_bytes()),
    );
    Ok(cache_dir()?
        .join(format!("{key:016x}"))
        .with_extension(MASK_EXTENSION))
}

/// The mask already cut for `image`, if there is one.
pub fn cached_mask(image: &Path) -> Option<PathBuf> {
    mask_path(image).ok().filter(|mask| mask.is_file())
}

fn cache_dir() -> Result<PathBuf, DepthCutError> {
    let base = env::var_os("XDG_CACHE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .ok_or(DepthCutError::NoCacheDir)?;
    Ok(base.join(CACHE_SUBDIR))
}

/// A hash that stays the same across builds and Rust versions, so every host
/// finds the masks any other cut.
fn fnv1a<'a>(bytes: impl Iterator<Item = &'a u8>) -> u64 {
    bytes.fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn the_key_follows_the_picture_not_the_build() {
        assert_eq!(fnv1a(b"".iter()), FNV_OFFSET);
        assert_eq!(fnv1a(b"a".iter()), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn a_changed_picture_gets_a_new_mask() -> Result<(), Box<dyn std::error::Error>> {
        let dir = env::temp_dir().join(format!("depthcut-test-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let picture = dir.join("wallpaper.jpg");
        fs::write(&picture, b"first")?;
        let first = mask_path(&picture)?;
        assert_eq!(first, mask_path(&picture)?);
        fs::write(&picture, b"second, longer")?;
        assert_ne!(first, mask_path(&picture)?);
        assert_eq!(first.extension(), Some(MASK_EXTENSION.as_ref()));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_missing_picture_has_no_mask() {
        assert!(cached_mask(Path::new("/nonexistent/wallpaper.jpg")).is_none());
    }
}

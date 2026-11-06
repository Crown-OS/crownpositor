use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    process,
};

/// Writes `contents` to `path` through a temporary file beside it, renamed
/// into place once it is complete and on disk, so a reader finds the old
/// file, no file or the whole new one: never part of one. Creates the
/// directory, and leaves no temporary behind on failure.
pub fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let temporary = temporary_beside(path);
    let written = write_then_rename(&temporary, path, contents);
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn write_then_rename(temporary: &Path, path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = File::create(temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}

/// A hidden name in the same directory, so the rename never crosses file
/// systems, unique to this process.
fn temporary_beside(path: &Path) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(".{}.tmp", process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        env::temp_dir().join(format!("depthcut-{name}-{}", process::id()))
    }

    fn entries(directory: &Path) -> io::Result<Vec<OsString>> {
        fs::read_dir(directory)?
            .map(|entry| Ok(entry?.file_name()))
            .collect()
    }

    #[test]
    fn it_creates_the_directory_and_replaces_the_file() -> io::Result<()> {
        let root = scratch("replace");
        let mask = root.join("depth").join("mask.png");
        write_atomically(&mask, b"first")?;
        write_atomically(&mask, b"second")?;
        assert_eq!(fs::read(&mask)?, b"second");
        assert_eq!(entries(&root.join("depth"))?, ["mask.png"]);
        fs::remove_dir_all(root)
    }

    #[test]
    fn a_failed_write_leaves_nothing_behind() -> io::Result<()> {
        let root = scratch("fail");
        let occupied = root.join("mask.png");
        fs::create_dir_all(&occupied)?;
        assert!(write_atomically(&occupied, b"mask").is_err());
        assert_eq!(entries(&root)?, ["mask.png"]);
        assert!(occupied.is_dir());
        fs::remove_dir_all(root)
    }
}

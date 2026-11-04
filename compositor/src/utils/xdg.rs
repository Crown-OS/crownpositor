//! The XDG base directories, in the order the spec searches them: the user's
//! own first, so what a user installed shadows the system's copy.

use std::path::PathBuf;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".local/share")))
}

fn system_data_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    dirs.split(':')
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".config")))
}

/// `subdir` under `$XDG_DATA_HOME`, then under every `$XDG_DATA_DIRS` entry.
pub fn data_dirs(subdir: &str) -> Vec<PathBuf> {
    data_home()
        .into_iter()
        .chain(system_data_dirs())
        .map(|dir| dir.join(subdir))
        .collect()
}

/// `$XDG_DATA_HOME/icons`, `~/.icons`, then `icons` under every
/// `$XDG_DATA_DIRS` entry: the order every icon and cursor loader uses.
pub fn icon_dirs() -> Vec<PathBuf> {
    data_home()
        .map(|dir| dir.join("icons"))
        .into_iter()
        .chain(home().map(|home| home.join(".icons")))
        .chain(system_data_dirs().into_iter().map(|dir| dir.join("icons")))
        .collect()
}

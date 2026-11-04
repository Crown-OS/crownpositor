//! Where the icon theme keeps an icon: the user's GTK icon theme, the themes
//! it inherits from, then hicolor and the flat pixmaps directory.
//!
//! Only application icons are looked for, so rather than reading every
//! theme's directory table this probes the few places themes put them, in
//! both layouts in use: `48x48/apps` (hicolor) and `apps/48` (breeze).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::utils::xdg;

/// Largest first: an icon is shown big, and shrinking reads better than
/// stretching.
const SIZES: [&str; 12] = [
    "scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48", "256", "128", "96",
    "64", "48",
];
const CONTEXT: &str = "apps";
const EXTENSIONS: [&str; 2] = ["svg", "png"];
const FALLBACK: &str = "hicolor";
const PIXMAPS: &str = "/usr/share/pixmaps";

pub fn find(name: &str) -> Option<PathBuf> {
    roots()
        .iter()
        .find_map(|root| {
            SIZES
                .iter()
                .flat_map(|size| [root.join(size).join(CONTEXT), root.join(CONTEXT).join(size)])
                .find_map(|dir| file_in(&dir, name))
        })
        .or_else(|| file_in(Path::new(PIXMAPS), name))
}

fn file_in(dir: &Path, name: &str) -> Option<PathBuf> {
    EXTENSIONS
        .iter()
        .map(|extension| dir.join(format!("{name}.{extension}")))
        .find(|path| path.is_file())
}

/// Every installed directory of the theme and what it inherits, in lookup
/// order.
fn roots() -> &'static [PathBuf] {
    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let dirs = xdg::icon_dirs();
        let mut themes = Vec::new();
        let mut pending = vec![gtk_icon_theme().unwrap_or_else(|| FALLBACK.to_owned())];
        while let Some(theme) = pending.pop() {
            if !themes.contains(&theme) {
                pending.extend(inherits(&dirs, &theme).into_iter().rev());
                themes.push(theme);
            }
        }
        if !themes.iter().any(|theme| theme == FALLBACK) {
            themes.push(FALLBACK.to_owned());
        }
        themes
            .iter()
            .flat_map(|theme| dirs.iter().map(move |dir| dir.join(theme)))
            .filter(|root| root.is_dir())
            .collect()
    })
}

fn inherits(dirs: &[PathBuf], theme: &str) -> Vec<String> {
    dirs.iter()
        .find_map(|dir| fs::read_to_string(dir.join(theme).join("index.theme")).ok())
        .and_then(|index| {
            index
                .lines()
                .find_map(|line| line.trim().strip_prefix("Inherits="))
                .map(|parents| {
                    parents
                        .split(',')
                        .map(str::trim)
                        .filter(|parent| !parent.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
        })
        .unwrap_or_default()
}

fn gtk_icon_theme() -> Option<String> {
    let config = xdg::config_home()?;
    ["gtk-4.0", "gtk-3.0"].iter().find_map(|gtk| {
        fs::read_to_string(config.join(gtk).join("settings.ini"))
            .ok()?
            .lines()
            .find_map(|line| line.trim().strip_prefix("gtk-icon-theme-name="))
            .map(|name| name.trim().trim_matches('"').to_owned())
            .filter(|name| !name.is_empty())
    })
}

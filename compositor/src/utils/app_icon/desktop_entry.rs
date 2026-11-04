//! Which icon an application's desktop entry names.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::utils::xdg;

const GROUP: &str = "[Desktop Entry]";
const EXTENSION: &str = "desktop";

/// The `Icon` of the entry named after `app_id`, or failing that of the entry
/// claiming `app_id` as its window class.
pub fn icon_name(app_id: &str) -> Option<String> {
    let lowered = app_id.to_lowercase();
    application_dirs()
        .iter()
        .flat_map(|dir| [app_id, &lowered].map(|name| dir.join(format!("{name}.{EXTENSION}"))))
        .find_map(|path| Entry::read(&path)?.icon)
        .or_else(|| by_window_class().get(&lowered).cloned())
}

fn application_dirs() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| xdg::data_dirs("applications"))
}

/// Every installed entry's icon by the lowercased window class it claims.
/// Read once, the first time an app id names no entry.
fn by_window_class() -> &'static HashMap<String, String> {
    static CLASSES: OnceLock<HashMap<String, String>> = OnceLock::new();
    CLASSES.get_or_init(|| {
        let mut classes = HashMap::new();
        let entries = application_dirs()
            .iter()
            .filter_map(|dir| fs::read_dir(dir).ok())
            .flatten()
            .filter_map(|file| Some(file.ok()?.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == EXTENSION)
            })
            .filter_map(|path| Entry::read(&path));
        for entry in entries {
            if let (Some(class), Some(icon)) = (entry.window_class, entry.icon) {
                classes.entry(class.to_lowercase()).or_insert(icon);
            }
        }
        classes
    })
}

#[derive(Debug, Default, PartialEq)]
struct Entry {
    icon: Option<String>,
    window_class: Option<String>,
}

impl Entry {
    fn read(path: &Path) -> Option<Self> {
        Some(Self::parse(&fs::read_to_string(path).ok()?))
    }

    fn parse(text: &str) -> Self {
        let mut entry = Self::default();
        let keys = text
            .lines()
            .map(str::trim)
            .skip_while(|line| *line != GROUP)
            .skip(1)
            .take_while(|line| !line.starts_with('['))
            .filter_map(|line| line.split_once('='));
        for (key, value) in keys {
            let value = Some(value.trim().to_owned()).filter(|value| !value.is_empty());
            match key.trim() {
                "Icon" => entry.icon = value,
                "StartupWMClass" => entry.window_class = value,
                _ => {}
            }
        }
        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_icon_and_class_of_the_main_group_only() {
        let entry = Entry::parse(
            "# comment\n[Desktop Entry]\nName=Files\nIcon=org.gnome.Nautilus\n\
             StartupWMClass=Nautilus\n[Desktop Action new]\nIcon=other\n",
        );
        assert_eq!(
            entry,
            Entry {
                icon: Some("org.gnome.Nautilus".into()),
                window_class: Some("Nautilus".into()),
            }
        );
    }

    #[test]
    fn an_empty_icon_is_no_icon() {
        assert_eq!(Entry::parse("[Desktop Entry]\nIcon=\n").icon, None);
    }
}

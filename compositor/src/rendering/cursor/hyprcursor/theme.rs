//! Finding a hyprcursor theme on disk and the shape archives inside it.

use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use zip::ZipArchive;

use crate::rendering::cursor::hyprcursor::meta::{Manifest, Meta};

const MANIFESTS: [&str; 2] = ["manifest.hl", "manifest.toml"];
const METAS: [&str; 2] = ["meta.hl", "meta.toml"];
const SHAPE_EXTENSION: &str = "hlc";

pub struct Theme {
    pub name: String,
    pub root: PathBuf,
    cursors: PathBuf,
    /// Shape name to archive, for names a shape claims through
    /// `define_override` rather than its file name. Built on the first name
    /// that has no archive of its own, since that means opening every archive.
    overrides: OnceLock<HashMap<String, PathBuf>>,
}

impl Theme {
    /// Matches a theme by directory name or by the name in its manifest, as
    /// libhyprcursor does, searching the user's icon directories first.
    pub fn find(name: &str) -> Option<Self> {
        let dirs = search_dirs();
        dirs.iter()
            .find_map(|dir| Self::load(&dir.join(name)))
            .or_else(|| {
                dirs.iter()
                    .filter_map(|dir| std::fs::read_dir(dir).ok())
                    .flatten()
                    .filter_map(Result::ok)
                    .find_map(|entry| Self::load(&entry.path()).filter(|theme| theme.name == name))
            })
    }

    pub fn load(root: &Path) -> Option<Self> {
        let manifest = MANIFESTS
            .iter()
            .find_map(|file| std::fs::read_to_string(root.join(file)).ok())
            .and_then(|text| Manifest::parse(&text))?;
        let name = manifest.name.unwrap_or_else(|| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        Some(Self {
            name,
            cursors: root.join(manifest.cursors_directory),
            root: root.to_owned(),
            overrides: OnceLock::new(),
        })
    }

    /// The archive that draws `shape`, either under its own name or as an
    /// override another shape declares.
    pub fn archive_for(&self, shape: &str) -> Option<PathBuf> {
        let own = self.cursors.join(shape).with_extension(SHAPE_EXTENSION);
        if own.is_file() {
            return Some(own);
        }
        self.overrides
            .get_or_init(|| self.index_overrides())
            .get(shape)
            .cloned()
    }

    fn index_overrides(&self) -> HashMap<String, PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.cursors) else {
            return HashMap::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == SHAPE_EXTENSION))
            .filter_map(|path| {
                let meta = ShapeArchive::open(&path)?.meta()?;
                Some(
                    meta.overrides
                        .into_iter()
                        .map(move |name| (name, path.clone())),
                )
            })
            .flatten()
            .collect()
    }
}

/// `$XDG_DATA_HOME/icons`, `~/.icons`, then `icons` under every
/// `$XDG_DATA_DIRS` entry: the order libhyprcursor and every XCursor loader
/// use, so a theme a user installed shadows the system's copy.
fn search_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());

    data_home
        .map(|dir| dir.join("icons"))
        .into_iter()
        .chain(home.map(|home| home.join(".icons")))
        .chain(
            data_dirs
                .split(':')
                .filter(|dir| !dir.is_empty())
                .map(|dir| Path::new(dir).join("icons")),
        )
        .collect()
}

/// One `.hlc` file: a zip of a `meta` file and the frame images it names.
pub struct ShapeArchive {
    zip: ZipArchive<File>,
}

impl ShapeArchive {
    pub fn open(path: &Path) -> Option<Self> {
        let zip = File::open(path)
            .map_err(zip::result::ZipError::from)
            .and_then(ZipArchive::new)
            .inspect_err(
                |err| tracing::warn!(%err, path = %path.display(), "unreadable hyprcursor shape"),
            )
            .ok()?;
        Some(Self { zip })
    }

    pub fn meta(&mut self) -> Option<Meta> {
        let text = METAS.iter().find_map(|name| self.read(name))?;
        Some(Meta::parse(&String::from_utf8_lossy(&text)))
    }

    pub fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        let mut file = self.zip.by_name(name).ok()?;
        let mut data = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut data).ok()?;
        Some(data)
    }
}
